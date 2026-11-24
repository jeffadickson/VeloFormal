use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Int(u64),
    SizedLit {
        width: u32,
        signed: bool,
        bits: String,
    },
    // Punctuation
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Colon,
    Comma,
    DotDot,
    Question,
    // Operators
    FatArrow, // =>
    ThinArrow, // ->
    OrOr,     // ||
    AndAnd,   // &&
    EqEq,     // ==
    NotEq,    // !=
    Le,       // <=
    Ge,       // >=
    Lt,       // <
    Gt,       // >
    Shl,      // <<
    Shr,      // >>
    Pipe,     // |
    Caret,    // ^
    Amp,      // &
    Bang,     // !
    Tilde,    // ~
    Plus,     // +
    Minus,    // -
    Star,     // *
    Slash,    // /
    Percent,  // %
    Eq,       // =
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut out = Vec::new();

    let advance = |i: &mut usize, line: &mut usize, col: &mut usize| -> u8 {
        let b = bytes[*i];
        *i += 1;
        if b == b'\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
        b
    };

    while i < bytes.len() {
        let b = bytes[i];
        // Whitespace
        if b.is_ascii_whitespace() {
            advance(&mut i, &mut line, &mut col);
            continue;
        }
        // Line comments: `// ...` or `# ...`
        if (b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/') || b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                advance(&mut i, &mut line, &mut col);
            }
            continue;
        }
        // Block comments: `/* ... */`
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start = Span { line, col };
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    advance(&mut i, &mut line, &mut col);
                    advance(&mut i, &mut line, &mut col);
                    closed = true;
                    break;
                }
                advance(&mut i, &mut line, &mut col);
            }
            if !closed {
                bail!("unterminated block comment starting at {start}");
            }
            continue;
        }

        let span = Span { line, col };

        // Identifiers (supporting dotted hierarchical paths like `u_sub.cnt`, NOT `1..8`)
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut s = String::new();
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    s.push(advance(&mut i, &mut line, &mut col) as char);
                } else if c == b'.'
                    && i + 1 < bytes.len()
                    && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
                {
                    s.push(advance(&mut i, &mut line, &mut col) as char);
                } else {
                    break;
                }
            }
            out.push(Token {
                kind: TokenKind::Ident(s),
                span,
            });
            continue;
        }

        // Numbers: `0x...`, `0b...`, SV sized literals `8'd3` / `8'sd5` / `4'hF` / `4'b1010`, or plain decimal
        if b.is_ascii_digit() {
            // Hex/bin prefix `0x` / `0b`
            if b == b'0' && i + 1 < bytes.len() && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B')
            {
                let base_ch = bytes[i + 1];
                advance(&mut i, &mut line, &mut col);
                advance(&mut i, &mut line, &mut col);
                let mut digits = String::new();
                while i < bytes.len()
                    && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_')
                {
                    let c = advance(&mut i, &mut line, &mut col) as char;
                    if c != '_' {
                        digits.push(c);
                    }
                }
                let radix = if base_ch == b'x' || base_ch == b'X' {
                    16
                } else {
                    2
                };
                let val = u64::from_str_radix(&digits, radix)
                    .map_err(|_| anyhow::anyhow!("invalid integer literal at {span}"))?;
                out.push(Token {
                    kind: TokenKind::Int(val),
                    span,
                });
                continue;
            }

            let mut num = String::new();
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                let c = advance(&mut i, &mut line, &mut col) as char;
                if c != '_' {
                    num.push(c);
                }
            }

            // Check for SystemVerilog sized literal `<width>'[s](b|d|h|o)<digits>`
            if i < bytes.len() && bytes[i] == b'\'' {
                let width: u32 = num
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid bitvector width '{num}' at {span}"))?;
                if width == 0 || width > 256 {
                    bail!("bitvector literal width {width} out of range 1..=256 at {span}");
                }
                advance(&mut i, &mut line, &mut col); // consume `'`
                let mut signed = false;
                if i < bytes.len() && matches!(bytes[i], b's' | b'S') {
                    signed = true;
                    advance(&mut i, &mut line, &mut col);
                }
                if i >= bytes.len() {
                    bail!("incomplete sized literal at {span}");
                }
                let base = advance(&mut i, &mut line, &mut col).to_ascii_lowercase();
                let mut digits = String::new();
                while i < bytes.len()
                    && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_')
                {
                    let c = advance(&mut i, &mut line, &mut col) as char;
                    if c != '_' {
                        digits.push(c);
                    }
                }
                if digits.is_empty() {
                    bail!("missing digits in sized literal at {span}");
                }
                let bits = parse_sized_bits(width, base, &digits, span)?;
                out.push(Token {
                    kind: TokenKind::SizedLit {
                        width,
                        signed,
                        bits,
                    },
                    span,
                });
                continue;
            }

            let val: u64 = num
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid integer literal '{num}' at {span}"))?;
            out.push(Token {
                kind: TokenKind::Int(val),
                span,
            });
            continue;
        }

        // Two-character tokens
        let next = bytes.get(i + 1).copied();
        let two = match (b, next) {
            (b'.', Some(b'.')) => Some(TokenKind::DotDot),
            (b'=', Some(b'>')) => Some(TokenKind::FatArrow),
            (b'-', Some(b'>')) => Some(TokenKind::ThinArrow),
            (b'|', Some(b'|')) => Some(TokenKind::OrOr),
            (b'&', Some(b'&')) => Some(TokenKind::AndAnd),
            (b'=', Some(b'=')) => Some(TokenKind::EqEq),
            (b'!', Some(b'=')) => Some(TokenKind::NotEq),
            (b'<', Some(b'=')) => Some(TokenKind::Le),
            (b'>', Some(b'=')) => Some(TokenKind::Ge),
            (b'<', Some(b'<')) => Some(TokenKind::Shl),
            (b'>', Some(b'>')) => Some(TokenKind::Shr),
            _ => None,
        };
        if let Some(kind) = two {
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            out.push(Token { kind, span });
            continue;
        }

        // Single-character tokens
        let one = match b {
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b';' => TokenKind::Semi,
            b':' => TokenKind::Colon,
            b',' => TokenKind::Comma,
            b'?' => TokenKind::Question,
            b'<' => TokenKind::Lt,
            b'>' => TokenKind::Gt,
            b'|' => TokenKind::Pipe,
            b'^' => TokenKind::Caret,
            b'&' => TokenKind::Amp,
            b'!' => TokenKind::Bang,
            b'~' => TokenKind::Tilde,
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            b'%' => TokenKind::Percent,
            b'=' => TokenKind::Eq,
            other => bail!("unexpected character '{}' at {span}", other as char),
        };
        advance(&mut i, &mut line, &mut col);
        out.push(Token { kind: one, span });
    }

    out.push(Token {
        kind: TokenKind::Eof,
        span: Span { line, col },
    });
    Ok(out)
}

fn parse_sized_bits(width: u32, base: u8, digits: &str, span: Span) -> Result<String> {
    let raw_bits = match base {
        b'b' => {
            if !digits.bytes().all(|c| c == b'0' || c == b'1') {
                bail!("invalid binary literal '{digits}' (4-state x/z not permitted in two-state literal) at {span}");
            }
            digits.to_string()
        }
        b'h' => {
            let mut s = String::with_capacity(digits.len() * 4);
            for ch in digits.chars() {
                let nib = ch
                    .to_digit(16)
                    .ok_or_else(|| anyhow::anyhow!("invalid hex digit '{ch}' at {span}"))?;
                s.push_str(&format!("{nib:04b}"));
            }
            s
        }
        b'o' => {
            let mut s = String::with_capacity(digits.len() * 3);
            for ch in digits.chars() {
                let oct = ch
                    .to_digit(8)
                    .ok_or_else(|| anyhow::anyhow!("invalid octal digit '{ch}' at {span}"))?;
                s.push_str(&format!("{oct:03b}"));
            }
            s
        }
        b'd' => {
            let val: u128 = digits
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid decimal literal '{digits}' at {span}"))?;
            if width < 128 && val >= (1u128 << width) {
                bail!(
                    "literal {width}'d{digits} overflows {width}-bit width at {span}"
                );
            }
            let mut s = String::with_capacity(width as usize);
            for bit in (0..width).rev() {
                let ch = if bit < 128 && ((val >> bit) & 1) != 0 {
                    '1'
                } else {
                    '0'
                };
                s.push(ch);
            }
            return Ok(s);
        }
        other => bail!("unsupported literal radix '{}' at {span}", other as char),
    };

    // Strip leading zeros to check overflow cleanly, then zero-pad to `width`.
    let trimmed = raw_bits.trim_start_matches('0');
    if trimmed.len() > width as usize {
        bail!("sized literal value overflows {width}-bit width at {span}");
    }
    let pad = width as usize - trimmed.len();
    Ok(format!("{}{}", "0".repeat(pad), trimmed))
}
