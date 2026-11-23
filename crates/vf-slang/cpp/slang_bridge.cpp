#include "slang_bridge.h"

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>
#include <unordered_map>
#include <vector>

#include "slang/ast/ASTContext.h"
#include "slang/ast/Compilation.h"
#include "slang/ast/EvalContext.h"
#include "slang/ast/Expression.h"
#include "slang/ast/Statement.h"
#include "slang/ast/SystemSubroutine.h"
#include "slang/ast/TimingControl.h"
#include "slang/ast/expressions/AssertionExpr.h"
#include "slang/ast/expressions/AssignmentExpressions.h"
#include "slang/ast/expressions/CallExpression.h"
#include "slang/ast/expressions/ConversionExpression.h"
#include "slang/ast/expressions/LiteralExpressions.h"
#include "slang/ast/expressions/MiscExpressions.h"
#include "slang/ast/expressions/OperatorExpressions.h"
#include "slang/ast/expressions/SelectExpressions.h"
#include "slang/ast/statements/ConditionalStatements.h"
#include "slang/ast/statements/LoopStatements.h"
#include "slang/ast/statements/MiscStatements.h"
#include "slang/ast/symbols/BlockSymbols.h"
#include "slang/ast/symbols/CompilationUnitSymbols.h"
#include "slang/ast/symbols/InstanceSymbols.h"
#include "slang/ast/symbols/MemberSymbols.h"
#include "slang/ast/symbols/PortSymbols.h"
#include "slang/ast/symbols/VariableSymbols.h"
#include "slang/ast/types/AllTypes.h"
#include "slang/diagnostics/TextDiagnosticClient.h"
#include "slang/driver/Driver.h"
#include "slang/numeric/SVInt.h"
#include "slang/text/Json.h"
#include "slang/text/SourceManager.h"

using namespace slang;
using namespace slang::ast;

namespace {

char* dupString(std::string_view sv) {
    char* out = static_cast<char*>(std::malloc(sv.size() + 1));
    if (!out)
        return nullptr;
    std::memcpy(out, sv.data(), sv.size());
    out[sv.size()] = '\0';
    return out;
}

// Yosys formal system functions: $initstate, $anyseq, $anyconst, $allseq, $allconst.
class FormalValueSubroutine final : public SystemSubroutine {
public:
    bool isBit;
    FormalValueSubroutine(const std::string& name, bool isBit) :
        SystemSubroutine(name, SubroutineKind::Function), isBit(isBit) {}

    const Type& checkArguments(const ASTContext& context, const Args& args, SourceRange range,
                               const Expression*) const override {
        auto& comp = context.getCompilation();
        if (!args.empty()) {
            return comp.getErrorType();
        }
        if (isBit)
            return comp.getLogicType();
        // $anyseq / $anyconst adapt to context width; return 32-bit unsigned integer by default
        // (slang converts to target width on assignment/cast).
        return comp.getIntType();
    }

    ConstantValue eval(EvalContext&, const Args&, SourceRange,
                       const CallExpression::SystemCallInfo&) const override {
        return nullptr;
    }
};

std::string svIntToBinary(const SVInt& val, uint32_t width) {
    std::string bits;
    bits.reserve(width);
    for (int32_t i = static_cast<int32_t>(width) - 1; i >= 0; --i) {
        auto b = val[i];
        bits.push_back(b.value == 1 ? '1' : '0');
    }
    return bits;
}

class ElaboratedEmitter {
public:
    Compilation& comp;
    const SourceManager& sm;
    JsonWriter& jw;
    std::unordered_map<const Symbol*, std::string> symNames;
    std::vector<std::string> errors;

    ElaboratedEmitter(Compilation& comp, const SourceManager& sm, JsonWriter& jw) :
        comp(comp), sm(sm), jw(jw) {}

    std::string formatLoc(SourceRange range) const {
        if (!range.start())
            return "";
        auto loc = range.start();
        auto file = sm.getFileName(loc);
        auto line = sm.getLineNumber(loc);
        auto col = sm.getColumnNumber(loc);
        return std::string(file) + ":" + std::to_string(line) + ":" + std::to_string(col);
    }

    std::string qualify(const std::string& prefix, std::string_view name) const {
        if (prefix.empty())
            return std::string(name);
        return prefix + "." + std::string(name);
    }

    void collectSymbols(const InstanceSymbol& inst, const std::string& prefix, bool isTop) {
        // First map ports so we know directions for top-level signals.
        std::unordered_map<const Symbol*, std::string> portDir;
        for (auto* portSym : inst.body.getPortList()) {
            if (auto* ps = portSym->as_if<PortSymbol>()) {
                if (ps->internalSymbol) {
                    std::string dir = "wire";
                    if (isTop) {
                        switch (ps->direction) {
                            case ArgumentDirection::In: dir = "input"; break;
                            case ArgumentDirection::Out: dir = "output"; break;
                            case ArgumentDirection::InOut: dir = "inout"; break;
                            default: dir = "wire"; break;
                        }
                    }
                    portDir[ps->internalSymbol] = dir;
                }
            }
        }

        collectScopeSymbols(inst.body, prefix, portDir);
    }

    void collectScopeSymbols(const Scope& scope, const std::string& prefix,
                             const std::unordered_map<const Symbol*, std::string>& portDir) {
        for (auto& member : scope.members()) {
            switch (member.kind) {
                case SymbolKind::Variable:
                case SymbolKind::Net: {
                    auto& vs = member.as<ValueSymbol>();
                    auto fullName = qualify(prefix, vs.name);
                    symNames[&vs] = fullName;
                    auto& ty = vs.getType().getCanonicalType();
                    if (ty.isIntegral()) {
                        auto it = portDir.find(&vs);
                        std::string dir = (it != portDir.end()) ? it->second : "wire";
                        bool isNet = (member.kind == SymbolKind::Net);
                        const Expression* init = vs.getInitializer();
                        if (isNet && init) {
                            portInputAssigns.emplace_back(fullName, init);
                            init = nullptr;
                        }
                        signals.push_back(SignalInfo{
                            fullName,
                            static_cast<uint32_t>(ty.getBitWidth()),
                            ty.isSigned(),
                            dir,
                            init,
                            &vs,
                        });
                    }
                    else if (ty.isUnpackedArray()) {
                        auto* elem = ty.getArrayElementType();
                        if (elem && elem->getCanonicalType().isIntegral()) {
                            auto range = ty.getFixedRange();
                            memories.push_back(MemInfo{
                                fullName,
                                static_cast<uint32_t>(elem->getCanonicalType().getBitWidth()),
                                static_cast<uint32_t>(range.width()),
                                range.lower(),
                                &vs,
                            });
                        }
                    }
                    break;
                }
                case SymbolKind::GenerateBlock: {
                    auto& gb = member.as<GenerateBlockSymbol>();
                    if (!gb.isUninstantiated) {
                        std::string subPrefix = gb.name.empty() ? prefix : qualify(prefix, gb.name);
                        collectScopeSymbols(gb, subPrefix, portDir);
                    }
                    break;
                }
                case SymbolKind::GenerateBlockArray: {
                    auto& gba = member.as<GenerateBlockArraySymbol>();
                    for (auto* entry : gba.entries) {
                        if (!entry->isUninstantiated) {
                            std::string idx = entry->arrayIndex ? entry->arrayIndex->toString(LiteralBase::Decimal, false) : "0";
                            std::string subPrefix = qualify(prefix, std::string(gba.name) + "[" + idx + "]");
                            collectScopeSymbols(*entry, subPrefix, portDir);
                        }
                    }
                    break;
                }
                case SymbolKind::Instance: {
                    auto& sub = member.as<InstanceSymbol>();
                    std::string subPrefix = qualify(prefix, sub.name);
                    collectSymbols(sub, subPrefix, false);
                    break;
                }
                default:
                    break;
            }
        }
    }

    struct SignalInfo {
        std::string name;
        uint32_t width;
        bool isSigned;
        std::string direction;
        const Expression* initializer;
        const ValueSymbol* sym;
    };

    struct MemInfo {
        std::string name;
        uint32_t dataWidth;
        uint32_t depth;
        int32_t loIndex;
        const ValueSymbol* sym;
    };

    struct AssignInfo {
        const Expression* lhs;
        const Expression* rhs;
        std::string loc;
    };

    struct ProcInfo {
        const ProceduralBlockSymbol* sym;
    };

    std::vector<SignalInfo> signals;
    std::vector<MemInfo> memories;
    std::vector<AssignInfo> continuousAssigns;
    std::vector<std::pair<std::string, const Expression*>> portInputAssigns;
    std::vector<std::pair<const Expression*, std::string>> portOutputAssigns;
    std::vector<ProcInfo> procedures;

    void collectBehavior(const InstanceSymbol& inst) {
        collectScopeBehavior(inst.body);
    }

    void collectScopeBehavior(const Scope& scope) {
        for (auto& member : scope.members()) {
            switch (member.kind) {
                case SymbolKind::ContinuousAssign: {
                    auto& ca = member.as<ContinuousAssignSymbol>();
                    auto& expr = ca.getAssignment();
                    if (auto* assign = expr.as_if<AssignmentExpression>()) {
                        continuousAssigns.push_back(AssignInfo{
                            &assign->left(),
                            &assign->right(),
                            formatLoc(assign->sourceRange),
                        });
                    }
                    break;
                }
                case SymbolKind::ProceduralBlock: {
                    procedures.push_back(ProcInfo{&member.as<ProceduralBlockSymbol>()});
                    break;
                }
                case SymbolKind::GenerateBlock: {
                    auto& gb = member.as<GenerateBlockSymbol>();
                    if (!gb.isUninstantiated)
                        collectScopeBehavior(gb);
                    break;
                }
                case SymbolKind::GenerateBlockArray: {
                    auto& gba = member.as<GenerateBlockArraySymbol>();
                    for (auto* entry : gba.entries) {
                        if (!entry->isUninstantiated)
                            collectScopeBehavior(*entry);
                    }
                    break;
                }
                case SymbolKind::Instance: {
                    auto& sub = member.as<InstanceSymbol>();
                    // Connect sub-instance ports to parent signals via continuous assignments.
                    for (auto* conn : sub.getPortConnections()) {
                        auto* ps = conn->port.as_if<PortSymbol>();
                        if (!ps || !ps->internalSymbol)
                            continue;
                        auto it = symNames.find(ps->internalSymbol);
                        if (it == symNames.end())
                            continue;
                        auto* extExpr = conn->getExpression();
                        if (!extExpr)
                            continue;
                        if (ps->direction == ArgumentDirection::In) {
                            portInputAssigns.emplace_back(it->second, extExpr);
                        }
                        else if (ps->direction == ArgumentDirection::Out) {
                            // For output ports, slang wraps the external target inside an
                            // AssignmentExpression (left = external, right = EmptyArgument).
                            if (auto* ae = extExpr->as_if<AssignmentExpression>()) {
                                portOutputAssigns.emplace_back(&ae->left(), it->second);
                            }
                            else {
                                portOutputAssigns.emplace_back(extExpr, it->second);
                            }
                        }
                    }
                    collectBehavior(sub);
                    break;
                }
                default:
                    break;
            }
        }
    }

    void emitDesign(const InstanceSymbol& top) {
        collectSymbols(top, "", true);
        collectBehavior(top);

        jw.startObject();
        jw.writeProperty("module");
        jw.writeValue(top.name);

        jw.writeProperty("signals");
        jw.startArray();
        for (auto& s : signals) {
            jw.startObject();
            jw.writeProperty("name"); jw.writeValue(s.name);
            jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(s.width));
            jw.writeProperty("signed"); jw.writeValue(s.isSigned);
            jw.writeProperty("direction"); jw.writeValue(s.direction);
            if (s.initializer) {
                jw.writeProperty("init");
                emitExpr(*s.initializer);
            }
            jw.endObject();
        }
        jw.endArray();

        jw.writeProperty("memories");
        jw.startArray();
        for (auto& m : memories) {
            jw.startObject();
            jw.writeProperty("name"); jw.writeValue(m.name);
            jw.writeProperty("data_width"); jw.writeValue(static_cast<uint64_t>(m.dataWidth));
            jw.writeProperty("depth"); jw.writeValue(static_cast<uint64_t>(m.depth));
            jw.writeProperty("lo_index"); jw.writeValue(static_cast<int64_t>(m.loIndex));
            jw.endObject();
        }
        jw.endArray();

        jw.writeProperty("assigns");
        jw.startArray();
        for (auto& a : continuousAssigns) {
            jw.startObject();
            jw.writeProperty("lhs"); emitExpr(*a.lhs);
            jw.writeProperty("rhs"); emitExpr(*a.rhs);
            jw.writeProperty("loc"); jw.writeValue(a.loc);
            jw.endObject();
        }
        for (auto& [internalName, extExpr] : portInputAssigns) {
            jw.startObject();
            jw.writeProperty("lhs");
            jw.startObject();
            jw.writeProperty("kind"); jw.writeValue(std::string_view("ref"));
            jw.writeProperty("name"); jw.writeValue(internalName);
            jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(extExpr->type->getBitWidth()));
            jw.writeProperty("signed"); jw.writeValue(extExpr->type->isSigned());
            jw.endObject();
            jw.writeProperty("rhs"); emitExpr(*extExpr);
            jw.writeProperty("loc"); jw.writeValue(std::string_view(""));
            jw.endObject();
        }
        for (auto& [extLhs, internalName] : portOutputAssigns) {
            jw.startObject();
            jw.writeProperty("lhs"); emitExpr(*extLhs);
            jw.writeProperty("rhs");
            jw.startObject();
            jw.writeProperty("kind"); jw.writeValue(std::string_view("ref"));
            jw.writeProperty("name"); jw.writeValue(internalName);
            jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(extLhs->type->getBitWidth()));
            jw.writeProperty("signed"); jw.writeValue(extLhs->type->isSigned());
            jw.endObject();
            jw.writeProperty("loc"); jw.writeValue(std::string_view(""));
            jw.endObject();
        }
        jw.endArray();

        jw.writeProperty("processes");
        jw.startArray();
        for (auto& p : procedures) {
            jw.startObject();
            std::string pkind = "always";
            switch (p.sym->procedureKind) {
                case ProceduralBlockKind::Initial: pkind = "initial"; break;
                case ProceduralBlockKind::Always: pkind = "always"; break;
                case ProceduralBlockKind::AlwaysComb: pkind = "always_comb"; break;
                case ProceduralBlockKind::AlwaysFF: pkind = "always_ff"; break;
                case ProceduralBlockKind::AlwaysLatch: pkind = "always_latch"; break;
                case ProceduralBlockKind::Final: pkind = "final"; break;
            }
            jw.writeProperty("kind"); jw.writeValue(pkind);
            jw.writeProperty("body");
            emitStmt(p.sym->getBody());
            jw.endObject();
        }
        jw.endArray();

        jw.endObject();
    }

    void emitTiming(const TimingControl& tc) {
        jw.startObject();
        switch (tc.kind) {
            case TimingControlKind::ImplicitEvent:
                jw.writeProperty("kind"); jw.writeValue(std::string_view("implicit"));
                break;
            case TimingControlKind::SignalEvent: {
                auto& se = tc.as<SignalEventControl>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("signal_event"));
                std::string edge = "none";
                switch (se.edge) {
                    case EdgeKind::PosEdge: edge = "posedge"; break;
                    case EdgeKind::NegEdge: edge = "negedge"; break;
                    case EdgeKind::BothEdges: edge = "edge"; break;
                    default: edge = "none"; break;
                }
                jw.writeProperty("edge"); jw.writeValue(edge);
                jw.writeProperty("expr"); emitExpr(se.expr);
                break;
            }
            case TimingControlKind::EventList: {
                auto& el = tc.as<EventListControl>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("event_list"));
                jw.writeProperty("events");
                jw.startArray();
                for (auto* ev : el.events)
                    emitTiming(*ev);
                jw.endArray();
                break;
            }
            default:
                jw.writeProperty("kind"); jw.writeValue(std::string_view("other"));
                break;
        }
        jw.endObject();
    }

    void emitAssertionExpr(const AssertionExpr& ae) {
        jw.startObject();
        switch (ae.kind) {
            case AssertionExprKind::Simple: {
                auto& s = ae.as<SimpleAssertionExpr>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("simple"));
                jw.writeProperty("expr"); emitExpr(s.expr);
                break;
            }
            case AssertionExprKind::Clocking: {
                auto& c = ae.as<ClockingAssertionExpr>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("clocking"));
                jw.writeProperty("clocking"); emitTiming(c.clocking);
                jw.writeProperty("expr"); emitAssertionExpr(c.expr);
                break;
            }
            case AssertionExprKind::DisableIff: {
                auto& d = ae.as<DisableIffAssertionExpr>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("disable_iff"));
                jw.writeProperty("cond"); emitExpr(d.condition);
                jw.writeProperty("expr"); emitAssertionExpr(d.expr);
                break;
            }
            case AssertionExprKind::Unary: {
                auto& u = ae.as<UnaryAssertionExpr>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("unary"));
                jw.writeProperty("op"); jw.writeValue(toString(u.op));
                jw.writeProperty("expr"); emitAssertionExpr(u.expr);
                break;
            }
            case AssertionExprKind::Binary: {
                auto& b = ae.as<BinaryAssertionExpr>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("binary"));
                jw.writeProperty("op"); jw.writeValue(toString(b.op));
                jw.writeProperty("left"); emitAssertionExpr(b.left);
                jw.writeProperty("right"); emitAssertionExpr(b.right);
                break;
            }
            default:
                jw.writeProperty("kind"); jw.writeValue(std::string_view("unsupported"));
                break;
        }
        jw.endObject();
    }

    void emitStmt(const Statement& stmt) {
        jw.startObject();
        switch (stmt.kind) {
            case StatementKind::Empty:
                jw.writeProperty("kind"); jw.writeValue(std::string_view("empty"));
                break;
            case StatementKind::List: {
                auto& list = stmt.as<StatementList>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("block"));
                jw.writeProperty("items");
                jw.startArray();
                for (auto* s : list.list)
                    emitStmt(*s);
                jw.endArray();
                break;
            }
            case StatementKind::Block: {
                auto& blk = stmt.as<BlockStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("block"));
                if (blk.blockSymbol && !blk.blockSymbol->name.empty()) {
                    jw.writeProperty("label"); jw.writeValue(blk.blockSymbol->name);
                }
                jw.writeProperty("items");
                jw.startArray();
                emitStmt(blk.body);
                jw.endArray();
                break;
            }
            case StatementKind::Timed: {
                auto& t = stmt.as<TimedStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("timed"));
                jw.writeProperty("timing"); emitTiming(t.timing);
                jw.writeProperty("body"); emitStmt(t.stmt);
                break;
            }
            case StatementKind::ExpressionStatement: {
                auto& es = stmt.as<ExpressionStatement>();
                if (auto* ae = es.expr.as_if<AssignmentExpression>()) {
                    jw.writeProperty("kind"); jw.writeValue(std::string_view(ae->isNonBlocking() ? "nba" : "assign"));
                    jw.writeProperty("lhs"); emitExpr(ae->left());
                    jw.writeProperty("rhs"); emitExpr(ae->right());
                }
                else {
                    jw.writeProperty("kind"); jw.writeValue(std::string_view("expr_stmt"));
                    jw.writeProperty("expr"); emitExpr(es.expr);
                }
                break;
            }
            case StatementKind::Conditional: {
                auto& cs = stmt.as<ConditionalStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("if"));
                jw.writeProperty("cond");
                if (!cs.conditions.empty()) {
                    emitExpr(*cs.conditions[0].expr);
                } else {
                    emitConst(1, "1", false);
                }
                jw.writeProperty("then"); emitStmt(cs.ifTrue);
                if (cs.ifFalse) {
                    jw.writeProperty("else"); emitStmt(*cs.ifFalse);
                }
                break;
            }
            case StatementKind::Case: {
                auto& cs = stmt.as<CaseStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("case"));
                jw.writeProperty("expr"); emitExpr(cs.expr);
                jw.writeProperty("items");
                jw.startArray();
                for (auto& item : cs.items) {
                    jw.startObject();
                    jw.writeProperty("exprs");
                    jw.startArray();
                    for (auto* e : item.expressions)
                        emitExpr(*e);
                    jw.endArray();
                    jw.writeProperty("body"); emitStmt(*item.stmt);
                    jw.endObject();
                }
                jw.endArray();
                if (cs.defaultCase) {
                    jw.writeProperty("default"); emitStmt(*cs.defaultCase);
                }
                break;
            }
            case StatementKind::ImmediateAssertion: {
                auto& ia = stmt.as<ImmediateAssertionStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("immediate_assert"));
                jw.writeProperty("assert_kind"); jw.writeValue(assertKindStr(ia.assertionKind));
                jw.writeProperty("cond"); emitExpr(ia.cond);
                jw.writeProperty("loc"); jw.writeValue(formatLoc(ia.sourceRange));
                break;
            }
            case StatementKind::ConcurrentAssertion: {
                auto& ca = stmt.as<ConcurrentAssertionStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("concurrent_assert"));
                jw.writeProperty("assert_kind"); jw.writeValue(assertKindStr(ca.assertionKind));
                jw.writeProperty("prop"); emitAssertionExpr(ca.propertySpec);
                jw.writeProperty("loc"); jw.writeValue(formatLoc(ca.sourceRange));
                break;
            }
            case StatementKind::ForLoop: {
                // Attempt static constant unrolling for bounded procedural loops.
                auto& fl = stmt.as<ForLoopStatement>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("for_loop"));
                jw.writeProperty("loc"); jw.writeValue(formatLoc(fl.sourceRange));
                break;
            }
            case StatementKind::VariableDeclaration: {
                jw.writeProperty("kind"); jw.writeValue(std::string_view("empty"));
                break;
            }
            default:
                jw.writeProperty("kind"); jw.writeValue(std::string_view("unsupported"));
                jw.writeProperty("loc"); jw.writeValue(formatLoc(stmt.sourceRange));
                break;
        }
        jw.endObject();
    }

    std::string assertKindStr(AssertionKind k) const {
        switch (k) {
            case AssertionKind::Assert: return "assert";
            case AssertionKind::Assume: return "assume";
            case AssertionKind::CoverProperty:
            case AssertionKind::CoverSequence: return "cover";
            case AssertionKind::Restrict: return "assume";
            default: return "assert";
        }
    }

    void emitConst(uint32_t width, const std::string& bits, bool isSigned) {
        jw.startObject();
        jw.writeProperty("kind"); jw.writeValue(std::string_view("const"));
        jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
        jw.writeProperty("signed"); jw.writeValue(isSigned);
        jw.writeProperty("bits"); jw.writeValue(bits);
        jw.endObject();
    }

    void emitExpr(const Expression& expr) {
        auto& ty = expr.type->getCanonicalType();
        uint32_t width = ty.isIntegral() ? static_cast<uint32_t>(ty.getBitWidth()) : 1;
        bool isSigned = ty.isIntegral() && ty.isSigned();

        // Fast-path compile-time constants (literals, parameters, localparams, enum values).
        if (expr.kind == ExpressionKind::IntegerLiteral) {
            auto& lit = expr.as<IntegerLiteral>();
            auto val = lit.getValue();
            emitConst(width, svIntToBinary(val, width), isSigned);
            return;
        }
        if (expr.kind == ExpressionKind::UnbasedUnsizedIntegerLiteral) {
            auto& lit = expr.as<UnbasedUnsizedIntegerLiteral>();
            auto val = lit.getValue();
            emitConst(width, svIntToBinary(val, width), isSigned);
            return;
        }
        if (expr.kind == ExpressionKind::NamedValue || expr.kind == ExpressionKind::HierarchicalValue) {
            auto& nv = expr.as<ValueExpressionBase>();
            if (nv.symbol.kind == SymbolKind::Parameter || nv.symbol.kind == SymbolKind::EnumValue ||
                nv.symbol.kind == SymbolKind::Genvar) {
                EvalContext evalCtx(ASTContext(comp.getRoot(), LookupLocation::max));
                auto cv = expr.eval(evalCtx);
                if (cv.isInteger()) {
                    emitConst(width, svIntToBinary(cv.integer(), width), isSigned);
                    return;
                }
            }
            auto it = symNames.find(&nv.symbol);
            std::string name = (it != symNames.end()) ? it->second : std::string(nv.symbol.name);
            jw.startObject();
            jw.writeProperty("kind"); jw.writeValue(std::string_view("ref"));
            jw.writeProperty("name"); jw.writeValue(name);
            jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
            jw.writeProperty("signed"); jw.writeValue(isSigned);
            jw.endObject();
            return;
        }

        jw.startObject();
        switch (expr.kind) {
            case ExpressionKind::Conversion: {
                auto& conv = expr.as<ConversionExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("conv"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("operand"); emitExpr(conv.operand());
                break;
            }
            case ExpressionKind::UnaryOp: {
                auto& u = expr.as<UnaryExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("unary"));
                jw.writeProperty("op"); jw.writeValue(toString(u.op));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("operand"); emitExpr(u.operand());
                break;
            }
            case ExpressionKind::BinaryOp: {
                auto& b = expr.as<BinaryExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("binary"));
                jw.writeProperty("op"); jw.writeValue(toString(b.op));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("left"); emitExpr(b.left());
                jw.writeProperty("right"); emitExpr(b.right());
                break;
            }
            case ExpressionKind::ConditionalOp: {
                auto& c = expr.as<ConditionalExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("ternary"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("cond"); emitExpr(*c.conditions[0].expr);
                jw.writeProperty("then"); emitExpr(c.left());
                jw.writeProperty("else"); emitExpr(c.right());
                break;
            }
            case ExpressionKind::Concatenation: {
                auto& cat = expr.as<ConcatenationExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("concat"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("operands");
                jw.startArray();
                for (auto* op : cat.operands())
                    emitExpr(*op);
                jw.endArray();
                break;
            }
            case ExpressionKind::Replication: {
                auto& rep = expr.as<ReplicationExpression>();
                EvalContext evalCtx(ASTContext(comp.getRoot(), LookupLocation::max));
                auto cv = rep.count().eval(evalCtx);
                uint64_t count = cv.isInteger() ? cv.integer().as<uint64_t>().value_or(1) : 1;
                jw.writeProperty("kind"); jw.writeValue(std::string_view("replicate"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("count"); jw.writeValue(count);
                jw.writeProperty("concat"); emitExpr(rep.concat());
                break;
            }
            case ExpressionKind::ElementSelect: {
                auto& es = expr.as<ElementSelectExpression>();
                auto& valTy = es.value().type->getCanonicalType();
                jw.writeProperty("kind");
                jw.writeValue(std::string_view(valTy.isUnpackedArray() ? "mem_select" : "bit_select"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("value"); emitExpr(es.value());
                jw.writeProperty("selector"); emitExpr(es.selector());
                if (valTy.isIntegral()) {
                    auto range = valTy.getFixedRange();
                    jw.writeProperty("range_left"); jw.writeValue(static_cast<int64_t>(range.left));
                    jw.writeProperty("range_right"); jw.writeValue(static_cast<int64_t>(range.right));
                }
                break;
            }
            case ExpressionKind::RangeSelect: {
                auto& rs = expr.as<RangeSelectExpression>();
                auto& valTy = rs.value().type->getCanonicalType();
                EvalContext evalCtx(ASTContext(comp.getRoot(), LookupLocation::max));
                auto lcv = rs.left().eval(evalCtx);
                auto rcv = rs.right().eval(evalCtx);
                jw.writeProperty("kind"); jw.writeValue(std::string_view("range_select"));
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("value"); emitExpr(rs.value());
                std::string selKind = "simple";
                if (rs.getSelectionKind() == RangeSelectionKind::IndexedUp) selKind = "indexed_up";
                else if (rs.getSelectionKind() == RangeSelectionKind::IndexedDown) selKind = "indexed_down";
                jw.writeProperty("sel_kind"); jw.writeValue(selKind);
                if (lcv.isInteger()) {
                    jw.writeProperty("left_const");
                    jw.writeValue(lcv.integer().as<int64_t>().value_or(0));
                } else {
                    jw.writeProperty("left"); emitExpr(rs.left());
                }
                if (rcv.isInteger()) {
                    jw.writeProperty("right_const");
                    jw.writeValue(rcv.integer().as<int64_t>().value_or(0));
                } else {
                    jw.writeProperty("right"); emitExpr(rs.right());
                }
                if (valTy.isIntegral()) {
                    auto range = valTy.getFixedRange();
                    jw.writeProperty("range_left"); jw.writeValue(static_cast<int64_t>(range.left));
                    jw.writeProperty("range_right"); jw.writeValue(static_cast<int64_t>(range.right));
                }
                break;
            }
            case ExpressionKind::Call: {
                auto& call = expr.as<CallExpression>();
                jw.writeProperty("kind"); jw.writeValue(std::string_view("call"));
                jw.writeProperty("name"); jw.writeValue(call.getSubroutineName());
                jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                jw.writeProperty("signed"); jw.writeValue(isSigned);
                jw.writeProperty("args");
                jw.startArray();
                for (auto* a : call.arguments())
                    emitExpr(*a);
                jw.endArray();
                break;
            }
            default: {
                // Fallback: try constant evaluation if possible.
                EvalContext evalCtx(ASTContext(comp.getRoot(), LookupLocation::max));
                auto cv = expr.eval(evalCtx);
                if (cv.isInteger()) {
                    jw.writeProperty("kind"); jw.writeValue(std::string_view("const"));
                    jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                    jw.writeProperty("signed"); jw.writeValue(isSigned);
                    jw.writeProperty("bits"); jw.writeValue(svIntToBinary(cv.integer(), width));
                } else {
                    jw.writeProperty("kind"); jw.writeValue(std::string_view("unsupported"));
                    jw.writeProperty("width"); jw.writeValue(static_cast<uint64_t>(width));
                    jw.writeProperty("signed"); jw.writeValue(isSigned);
                }
                break;
            }
        }
        jw.endObject();
    }
};

} // namespace

extern "C" int32_t vf_slang_elaborate(
    int32_t argc,
    const char* const* argv,
    char** out_json,
    char** out_diag) {
    if (out_json) *out_json = nullptr;
    if (out_diag) *out_diag = nullptr;

    try {
        slang::driver::Driver driver;
        driver.addStandardArgs();
        if (!driver.parseCommandLine(argc, argv)) {
            if (out_diag) *out_diag = dupString("Failed to parse slang arguments");
            return 1;
        }
        if (!driver.processOptions()) {
            if (out_diag) *out_diag = dupString("Failed to process slang options");
            return 1;
        }
        if (!driver.parseAllSources()) {
            if (out_diag) *out_diag = dupString(driver.textDiagClient->getString());
            return 1;
        }

        auto compilation = driver.createCompilation();

        // Register Yosys formal verification system functions on the compilation.
        compilation->addSystemSubroutine(std::make_shared<FormalValueSubroutine>("$initstate", true));
        compilation->addSystemSubroutine(std::make_shared<FormalValueSubroutine>("$anyseq", false));
        compilation->addSystemSubroutine(std::make_shared<FormalValueSubroutine>("$anyconst", false));
        compilation->addSystemSubroutine(std::make_shared<FormalValueSubroutine>("$allseq", false));
        compilation->addSystemSubroutine(std::make_shared<FormalValueSubroutine>("$allconst", false));

        // Force full elaboration and collect all diagnostics.
        auto& root = compilation->getRoot();
        auto& diags = compilation->getAllDiagnostics();

        bool hasErrors = false;
        for (auto& d : diags) {
            if (driver.diagEngine.getSeverity(d.code, d.location) >= DiagnosticSeverity::Error) {
                hasErrors = true;
            }
        }

        driver.diagEngine.clearCounts();
        for (auto& d : diags) {
            driver.diagEngine.issue(d);
        }
        std::string diagText = driver.textDiagClient->getString();
        if (out_diag && !diagText.empty()) {
            *out_diag = dupString(diagText);
        }

        if (hasErrors) {
            return 2;
        }

        if (root.topInstances.empty()) {
            if (out_diag) *out_diag = dupString("No top-level module found in Verilog sources");
            return 3;
        }

        const InstanceSymbol* top = root.topInstances[0];
        JsonWriter jw;
        ElaboratedEmitter emitter(*compilation, driver.sourceManager, jw);
        emitter.emitDesign(*top);

        if (out_json) {
            *out_json = dupString(jw.view());
        }
        return 0;
    }
    catch (const std::exception& e) {
        if (out_diag) *out_diag = dupString(e.what());
        return 99;
    }
}

extern "C" void vf_slang_free_string(char* ptr) {
    std::free(ptr);
}
