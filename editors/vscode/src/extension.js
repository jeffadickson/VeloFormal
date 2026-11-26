const vscode = require("vscode");
const cp = require("child_process");
const path = require("path");

let lspProc = null;

function activate(context) {
  const cfg = vscode.workspace.getConfiguration("veloformal");
  const cliPath = cfg.get("cliPath", "veloformal");
  const lspPath = cfg.get("lspPath", "vf-lsp");
  const diagCollection = vscode.languages.createDiagnosticCollection("veloformal");
  context.subscriptions.push(diagCollection);

  // Spawn native Rust `vf-lsp` over stdio JSON-RPC 2.0
  let nextId = 1;
  let buffer = Buffer.alloc(0);
  const pending = new Map();

  function send(msg) {
    if (!lspProc || !lspProc.stdin.writable) return;
    const body = Buffer.from(JSON.stringify(msg), "utf8");
    const hdr = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "utf8");
    lspProc.stdin.write(Buffer.concat([hdr, body]));
  }

  function request(method, params) {
    const id = nextId++;
    return new Promise((resolve) => {
      pending.set(id, resolve);
      send({ jsonrpc: "2.0", id, method, params });
    });
  }

  try {
    lspProc = cp.spawn(lspPath, [], { stdio: ["pipe", "pipe", "ignore"] });
    lspProc.stdout.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      while (true) {
        const sep = buffer.indexOf("\r\n\r\n");
        if (sep < 0) break;
        const header = buffer.slice(0, sep).toString("utf8");
        const m = /Content-Length:\s*(\d+)/i.exec(header);
        if (!m) break;
        const len = parseInt(m[1], 10);
        if (buffer.length < sep + 4 + len) break;
        const payload = buffer.slice(sep + 4, sep + 4 + len).toString("utf8");
        buffer = buffer.slice(sep + 4 + len);
        const msg = JSON.parse(payload);
        if (msg.id && pending.has(msg.id)) {
          pending.get(msg.id)(msg.result);
          pending.delete(msg.id);
        } else if (msg.method === "textDocument/publishDiagnostics") {
          const uri = vscode.Uri.parse(msg.params.uri);
          const list = (msg.params.diagnostics || []).map((d) => {
            const r = new vscode.Range(d.range.start.line, d.range.start.character, d.range.end.line, d.range.end.character);
            return new vscode.Diagnostic(r, d.message, vscode.DiagnosticSeverity.Error);
          });
          diagCollection.set(uri, list);
        }
      }
    });
    request("initialize", { processId: process.pid, rootUri: null, capabilities: {} }).then(() => {
      send({ jsonrpc: "2.0", method: "initialized", params: {} });
      vscode.workspace.textDocuments.forEach((doc) => {
        if (doc.languageId === "vfc") {
          send({ jsonrpc: "2.0", method: "textDocument/didOpen", params: { textDocument: { uri: doc.uri.toString(), languageId: "vfc", version: doc.version, text: doc.getText() } } });
        }
      });
    });
  } catch (_) {}

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => {
      if (doc.languageId === "vfc") {
        send({ jsonrpc: "2.0", method: "textDocument/didOpen", params: { textDocument: { uri: doc.uri.toString(), languageId: "vfc", version: doc.version, text: doc.getText() } } });
      }
    }),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (e.document.languageId === "vfc") {
        send({ jsonrpc: "2.0", method: "textDocument/didChange", params: { textDocument: { uri: e.document.uri.toString(), version: e.document.version }, contentChanges: [{ text: e.document.getText() }] } });
      }
    }),
    vscode.languages.registerDocumentFormattingEditProvider("vfc", {
      async provideDocumentFormattingEdits(doc) {
        const edits = await request("textDocument/formatting", { textDocument: { uri: doc.uri.toString() } });
        return (edits || []).map((e) =>
          vscode.TextEdit.replace(new vscode.Range(e.range.start.line, e.range.start.character, e.range.end.line, e.range.end.character), e.newText)
        );
      },
    }),
    vscode.languages.registerHoverProvider("vfc", {
      async provideHover(doc, pos) {
        const res = await request("textDocument/hover", { textDocument: { uri: doc.uri.toString() }, position: { line: pos.line, character: pos.character } });
        return res && res.contents ? new vscode.Hover(new vscode.MarkdownString(res.contents.value)) : null;
      },
    }),
    vscode.commands.registerCommand("veloformal.verifyContract", (uriStr) => {
      const file = uriStr ? vscode.Uri.parse(uriStr).fsPath : vscode.window.activeTextEditor?.document.fileName;
      if (!file) return;
      const dir = path.dirname(file);
      const term = vscode.window.createTerminal("VeloFormal");
      term.show();
      term.sendText(`${cliPath} contract "${dir}"/*.sv --spec "${file}" --vacuity`);
    }),
    vscode.commands.registerCommand("veloformal.emitSva", (uriStr) => {
      const file = uriStr ? vscode.Uri.parse(uriStr).fsPath : vscode.window.activeTextEditor?.document.fileName;
      if (!file) return;
      const term = vscode.window.createTerminal("VeloFormal");
      term.show();
      term.sendText(`${cliPath} emit-sva "${file}"`);
    }),
    vscode.commands.registerCommand("veloformal.openWaveformDebugger", async (uriStr) => {
      const fs = require("fs");
      let htmlPath = uriStr ? vscode.Uri.parse(uriStr).fsPath : vscode.window.activeTextEditor?.document.fileName;
      if (!htmlPath || !htmlPath.endsWith(".html")) {
        const picked = await vscode.window.showOpenDialog({
          canSelectMany: false,
          filters: { "VeloFormal HTML Report": ["html"] },
          openLabel: "Open VeloFormal Waveform Debugger",
        });
        if (!picked || picked.length === 0) return;
        htmlPath = picked[0].fsPath;
      }
      const panel = vscode.window.createWebviewPanel(
        "veloformalDebugger",
        `VeloFormal Debugger — ${path.basename(htmlPath)}`,
        vscode.ViewColumn.Beside,
        { enableScripts: true }
      );
      panel.webview.html = fs.readFileSync(htmlPath, "utf8");
    }),
    vscode.commands.registerCommand("veloformal.runAutocheck", async (uriStr) => {
      const fs = require("fs");
      const os = require("os");
      const file = uriStr ? vscode.Uri.parse(uriStr).fsPath : vscode.window.activeTextEditor?.document.fileName;
      if (!file) return;
      const reportPath = path.join(os.tmpdir(), `veloformal_autocheck_${Date.now()}.html`);
      cp.execFile(
        cliPath,
        ["autocheck", file, "--auto-fsm", "--report", reportPath],
        (_err, _stdout, stderr) => {
          if (fs.existsSync(reportPath)) {
            const panel = vscode.window.createWebviewPanel(
              "veloformalAutocheck",
              `VeloFormal AutoCheck — ${path.basename(file)}`,
              vscode.ViewColumn.Beside,
              { enableScripts: true }
            );
            panel.webview.html = fs.readFileSync(reportPath, "utf8");
          } else {
            vscode.window.showErrorMessage(`VeloFormal AutoCheck failed: ${stderr}`);
          }
        }
      );
    })
  );
}

function deactivate() {
  if (lspProc) lspProc.kill();
}

module.exports = { activate, deactivate };
