//! The part of Go's text/template that chezmoi repos commonly use: text,
//! `if`/`else if`/`else`/`end`, output of a pipeline, and comments. Anything
//! else is an error naming its line, so a file is converted whole or not at
//! all.

/// A template, with Go's `{{-` and `-}}` already applied to the text.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Text(String),
    Output {
        expr: Expr,
        line: usize,
    },
    If {
        branches: Vec<(Expr, Vec<Node>)>,
        otherwise: Option<Vec<Node>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `.a.b`; empty for `.` itself.
    Field(Vec<String>),
    Str(String),
    Num(String),
    Bool(bool),
    /// A function and its arguments; a piped value is the last argument.
    Call(String, Vec<Expr>),
    /// Fields of a parenthesized value: `(f "x").a.b`.
    Chain(Box<Expr>, Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub line: usize,
    pub what: String,
}

type Parsed<T> = Result<T, Unsupported>;

fn unsupported<T>(line: usize, what: impl Into<String>) -> Parsed<T> {
    Err(Unsupported {
        line,
        what: what.into(),
    })
}

/// One `{{ }}` and where it starts.
struct Action {
    body: String,
    line: usize,
}

enum Piece {
    Text(String),
    Action(Action),
}

/// The white space Go's trim markers remove: not the rest of Unicode's.
const SPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Text and actions, the trim markers applied.
fn split(source: &str) -> Parsed<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut rest = source;
    let mut line = 1;
    let mut trim_next = false;
    while let Some(start) = rest.find("{{") {
        let mut text = &rest[..start];
        if trim_next {
            text = text.trim_start_matches(SPACE);
        }
        let after = &rest[start + 2..];
        let trim_before = after.starts_with('-') && after[1..].starts_with(SPACE);
        if trim_before {
            text = text.trim_end_matches(SPACE);
        }
        line += rest[..start].matches('\n').count();
        let inner_start = if trim_before { 1 } else { 0 };
        let Some(end) = find_close(&after[inner_start..]) else {
            return unsupported(line, "an action without `}}`");
        };
        let inner = &after[inner_start..inner_start + end];
        let (body, trim_after) = match inner.strip_suffix('-') {
            Some(body) if body.ends_with(SPACE) => (body, true),
            _ => (inner, false),
        };
        if !text.is_empty() {
            pieces.push(Piece::Text(text.to_string()));
        }
        pieces.push(Piece::Action(Action {
            body: body.trim_matches(SPACE).to_string(),
            line,
        }));
        line += inner.matches('\n').count();
        rest = &after[inner_start + end + 2..];
        trim_next = trim_after;
    }
    let text = if trim_next {
        rest.trim_start_matches(SPACE)
    } else {
        rest
    };
    if !text.is_empty() {
        pieces.push(Piece::Text(text.to_string()));
    }
    Ok(pieces)
}

/// Where the action's `}}` is, past any quoted string.
fn find_close(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'`' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    i += 1;
                }
            }
            b'}' if bytes[i + 1] == b'}' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

pub fn parse(source: &str) -> Parsed<Vec<Node>> {
    let pieces = split(source)?;
    let mut iter = pieces.into_iter().peekable();
    let (nodes, end) = block(&mut iter)?;
    match end {
        None => Ok(nodes),
        Some((_, line)) => unsupported(line, "`else` or `end` without `if`"),
    }
}

/// What ended a block: `else`, `else if <cond>` or `end`, with its line.
type BlockEnd = Option<(Option<Option<Expr>>, usize)>;

fn block(
    pieces: &mut std::iter::Peekable<std::vec::IntoIter<Piece>>,
) -> Parsed<(Vec<Node>, BlockEnd)> {
    let mut nodes = Vec::new();
    while let Some(piece) = pieces.next() {
        let action = match piece {
            Piece::Text(text) => {
                nodes.push(Node::Text(text));
                continue;
            }
            Piece::Action(action) => action,
        };
        let line = action.line;
        let body = action.body.as_str();
        if body.starts_with("/*") {
            if !body.ends_with("*/") {
                return unsupported(line, "a comment without `*/`");
            }
            continue;
        }
        let tokens = lex(body, line)?;
        match tokens.first() {
            Some(Tok::Ident(word)) if word == "if" => {
                let cond = expression(&tokens[1..], line)?;
                nodes.push(if_block(cond, pieces, line)?);
            }
            Some(Tok::Ident(word)) if word == "else" => {
                let rest = &tokens[1..];
                let branch = match rest.first() {
                    None => None,
                    Some(Tok::Ident(word)) if word == "if" => Some(expression(&rest[1..], line)?),
                    _ => return unsupported(line, format!("`{body}`")),
                };
                return Ok((nodes, Some((Some(branch), line))));
            }
            Some(Tok::Ident(word)) if word == "end" && tokens.len() == 1 => {
                return Ok((nodes, Some((None, line))));
            }
            Some(Tok::Ident(word)) if KEYWORDS.contains(&word.as_str()) => {
                return unsupported(line, format!("`{word}`"));
            }
            _ => nodes.push(Node::Output {
                expr: expression(&tokens, line)?,
                line,
            }),
        }
    }
    Ok((nodes, None))
}

const KEYWORDS: &[&str] = &[
    "range", "with", "template", "define", "block", "break", "continue", "end", "else",
];

fn if_block(
    first: Expr,
    pieces: &mut std::iter::Peekable<std::vec::IntoIter<Piece>>,
    line: usize,
) -> Parsed<Node> {
    let mut branches = Vec::new();
    let mut cond = first;
    loop {
        let (nodes, end) = block(pieces)?;
        match end {
            None => return unsupported(line, "`if` without `end`"),
            Some((None, _)) => {
                branches.push((cond, nodes));
                return Ok(Node::If {
                    branches,
                    otherwise: None,
                });
            }
            Some((Some(Some(next)), _)) => {
                branches.push((cond, nodes));
                cond = next;
            }
            Some((Some(None), _)) => {
                branches.push((cond, nodes));
                let (otherwise, end) = block(pieces)?;
                return match end {
                    Some((None, _)) => Ok(Node::If {
                        branches,
                        otherwise: Some(otherwise),
                    }),
                    _ => unsupported(line, "`if` without `end`"),
                };
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Field(Vec<String>),
    /// Fields right after `)`: `(f).a`, unlike the argument in `(f) .a`.
    Chain(Vec<String>),
    Ident(String),
    Str(String),
    Num(String),
    Open,
    Close,
    Pipe,
}

fn lex(body: &str, line: usize) -> Parsed<Vec<Tok>> {
    let chars: Vec<char> = body.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let word = |i: &mut usize| {
        let start = *i;
        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
            *i += 1;
        }
        chars[start..*i].iter().collect::<String>()
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                tokens.push(Tok::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::Close);
                i += 1;
            }
            '|' => {
                tokens.push(Tok::Pipe);
                i += 1;
            }
            '.' => {
                let chained = i > 0 && chars[i - 1] == ')';
                let mut path = Vec::new();
                while i < chars.len() && chars[i] == '.' {
                    i += 1;
                    let name = word(&mut i);
                    if name.is_empty() {
                        break;
                    }
                    path.push(name);
                }
                tokens.push(if chained {
                    Tok::Chain(path)
                } else {
                    Tok::Field(path)
                });
            }
            '"' => {
                let mut text = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return unsupported(line, "an unterminated string"),
                        Some('"') => break,
                        Some('\\') => {
                            i += 1;
                            text.push(match chars.get(i) {
                                Some('n') => '\n',
                                Some('t') => '\t',
                                Some('\\') => '\\',
                                Some('"') => '"',
                                _ => return unsupported(line, "a string escape"),
                            });
                        }
                        Some(&c) => text.push(c),
                    }
                    i += 1;
                }
                i += 1;
                tokens.push(Tok::Str(text));
            }
            '`' => {
                let start = i + 1;
                let Some(len) = chars[start..].iter().position(|&c| c == '`') else {
                    return unsupported(line, "an unterminated string");
                };
                tokens.push(Tok::Str(chars[start..start + len].iter().collect()));
                i = start + len + 1;
            }
            c if c.is_ascii_digit()
                || (c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) =>
            {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                tokens.push(Tok::Num(chars[start..i].iter().collect()));
            }
            c if c.is_alphabetic() || c == '_' => tokens.push(Tok::Ident(word(&mut i))),
            '$' => return unsupported(line, "variables"),
            _ => return unsupported(line, format!("`{c}`")),
        }
    }
    Ok(tokens)
}

fn expression(tokens: &[Tok], line: usize) -> Parsed<Expr> {
    let (expr, used) = pipeline(tokens, line)?;
    if used != tokens.len() {
        return unsupported(line, "an unexpected `)`");
    }
    Ok(expr)
}

/// `command ('|' command)*`, up to an unmatched `)`.
fn pipeline(tokens: &[Tok], line: usize) -> Parsed<(Expr, usize)> {
    let (mut expr, mut used) = command(tokens, None, line)?;
    while tokens.get(used) == Some(&Tok::Pipe) {
        let (next, more) = command(&tokens[used + 1..], Some(expr), line)?;
        expr = next;
        used += 1 + more;
    }
    Ok((expr, used))
}

/// A function call or a single operand; `piped` joins as the last argument.
fn command(tokens: &[Tok], piped: Option<Expr>, line: usize) -> Parsed<(Expr, usize)> {
    let mut operands = Vec::new();
    let mut used = 0;
    while let Some(token) = tokens.get(used) {
        if matches!(token, Tok::Pipe | Tok::Close) {
            break;
        }
        let (operand, more) = operand(&tokens[used..], line)?;
        operands.push(operand);
        used += more;
    }
    let mut operands = operands.into_iter();
    let Some(first) = operands.next() else {
        return unsupported(line, "an empty command");
    };
    let mut args: Vec<Expr> = operands.collect();
    let expr = match first {
        Expr::Call(name, empty) if empty.is_empty() => {
            args.extend(piped);
            Expr::Call(name, args)
        }
        first if args.is_empty() && piped.is_none() => first,
        _ => return unsupported(line, "arguments to a value"),
    };
    Ok((expr, used))
}

fn operand(tokens: &[Tok], line: usize) -> Parsed<(Expr, usize)> {
    let (expr, mut used) = match &tokens[0] {
        Tok::Field(path) => (Expr::Field(path.clone()), 1),
        Tok::Str(text) => (Expr::Str(text.clone()), 1),
        Tok::Num(number) => (Expr::Num(number.clone()), 1),
        Tok::Ident(word) if word == "true" || word == "false" => (Expr::Bool(word == "true"), 1),
        Tok::Ident(word) if word == "nil" || KEYWORDS.contains(&word.as_str()) => {
            return unsupported(line, format!("`{word}`"));
        }
        Tok::Ident(word) => (Expr::Call(word.clone(), Vec::new()), 1),
        Tok::Open => {
            let (inner, more) = pipeline(&tokens[1..], line)?;
            if tokens.get(1 + more) != Some(&Tok::Close) {
                return unsupported(line, "a `(` without `)`");
            }
            (inner, more + 2)
        }
        Tok::Close | Tok::Pipe | Tok::Chain(_) => {
            return unsupported(line, "an unexpected `)`, `|` or field");
        }
    };
    if let Some(Tok::Chain(path)) = tokens.get(used) {
        used += 1;
        return Ok((Expr::Chain(Box::new(expr), path.clone()), used));
    }
    Ok((expr, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(path: &str) -> Expr {
        Expr::Field(path.split('.').map(str::to_string).collect())
    }

    #[test]
    fn trims_and_comments_follow_go() {
        let nodes = parse("a  \n{{- /* note */ -}}\n  b {{ .x }}\n").unwrap();
        assert_eq!(
            nodes,
            [
                Node::Text("a".into()),
                Node::Text("b ".into()),
                Node::Output {
                    expr: field("x"),
                    line: 3
                },
                Node::Text("\n".into()),
            ]
        );
        // Go trims only ASCII space, tab and line ends, as chezmoi 2.74 does.
        assert_eq!(
            parse("a\u{3000}{{- \"x\" -}}\u{a0}b\u{c}{{- .y }}").unwrap(),
            [
                Node::Text("a\u{3000}".into()),
                Node::Output {
                    expr: Expr::Str("x".into()),
                    line: 1
                },
                Node::Text("\u{a0}b\u{c}".into()),
                Node::Output {
                    expr: field("y"),
                    line: 1
                },
            ]
        );
        // `{{-3}}` is the number -3, not a trim marker.
        assert_eq!(
            parse("{{-3}}").unwrap(),
            [Node::Output {
                expr: Expr::Num("-3".into()),
                line: 1
            }]
        );
    }

    #[test]
    fn conditions_nest() {
        let nodes = parse(
            "{{ if eq .chezmoi.os \"darwin\" }}mac{{ else if (and (ne .a \"x\") .b) }}x{{ else }}other{{ end }}",
        )
        .unwrap();
        let Node::If {
            branches,
            otherwise,
        } = &nodes[0]
        else {
            panic!("{nodes:?}");
        };
        assert_eq!(
            branches[0].0,
            Expr::Call(
                "eq".into(),
                vec![field("chezmoi.os"), Expr::Str("darwin".into())]
            )
        );
        assert_eq!(
            branches[1].0,
            Expr::Call(
                "and".into(),
                vec![
                    Expr::Call("ne".into(), vec![field("a"), Expr::Str("x".into())]),
                    field("b")
                ]
            )
        );
        assert_eq!(otherwise, &Some(vec![Node::Text("other".into())]));
    }

    #[test]
    fn pipes_and_chains() {
        let nodes = parse("{{ include \"dot_Brewfile\" | sha256sum }}{{ (bitwarden \"item\" \"gh\").login.password }}").unwrap();
        assert_eq!(
            nodes[0],
            Node::Output {
                expr: Expr::Call(
                    "sha256sum".into(),
                    vec![Expr::Call(
                        "include".into(),
                        vec![Expr::Str("dot_Brewfile".into())]
                    )]
                ),
                line: 1
            }
        );
        assert_eq!(
            nodes[1],
            Node::Output {
                expr: Expr::Chain(
                    Box::new(Expr::Call(
                        "bitwarden".into(),
                        vec![Expr::Str("item".into()), Expr::Str("gh".into())]
                    )),
                    vec!["login".into(), "password".into()]
                ),
                line: 1
            }
        );
    }

    #[test]
    fn the_rest_names_its_line() {
        for (source, line) in [
            ("a\n{{ range .x }}{{ end }}", 2),
            ("a\nb\n{{ $x := 1 }}", 3),
            ("{{ if .a }}", 1),
            ("x\n{{ end }}", 2),
            ("{{ template \"t\" . }}", 1),
        ] {
            assert_eq!(parse(source).unwrap_err().line, line, "{source}");
        }
        // A `}}` inside a string does not end the action.
        assert!(parse("{{ \"}}\" }}").is_ok());
    }
}
