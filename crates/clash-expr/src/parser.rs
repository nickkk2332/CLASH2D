//! The parser: source text to a syntax tree (05 §4.1), and text templates
//! to their parts (05 §4.6).
//!
//! The grammar is the one in the specification, with one production for
//! each function here:
//!
//! ```text
//! expr     := 'if' expr 'then' expr 'else' expr | or
//! or       := and  ( 'or' and )*
//! and      := not  ( 'and' not )*
//! not      := 'not' not | compare
//! compare  := coalesce ( ( '==' | '!=' | '<' | '<=' | '>' | '>=' | 'in' ) coalesce )?
//! coalesce := sum ( '??' sum )*
//! sum      := term ( ( '+' | '-' ) term )*
//! term     := unary ( ( '*' | '/' | '%' ) unary )*
//! unary    := '-' unary | power
//! power    := postfix ( '^' unary )?
//! postfix  := primary ( '.' name | '(' args? ')' | '[' expr ']' )*
//! primary  := number unit? | string | 'true' | 'false' | 'null'
//!           | name | '$' name | '(' expr ')' | '[' args? ']'
//!           | '{' ( entry ( ',' entry )* )? '}'
//! args     := expr ( ',' expr )*
//! entry    := expr ':' expr
//! name     := ident ( ':' ident )?
//! ```
//!
//! A syntax error is one [`Diagnostic`] whose span is the offending bytes
//! of the source. Parsing stops at the first error in an expression; a
//! text template reports one for each of its expressions.

use crate::ast::{
    BinaryOp, Expr, ExprKind, Format, MAX_FORMAT_PRECISION, Name, Template, TemplatePart, UnaryOp,
};
use crate::diag::{Diagnostic, Span, codes};
use crate::lexer::{Lexer, Tok, Token};
use crate::unit::Unit;

/// The deepest a syntax tree may be. A longer chain of operators or a
/// deeper nesting is refused, so that no expression a mod can write
/// exhausts the stack of anything that walks the tree.
pub const MAX_TREE_HEIGHT: u32 = 100;

/// The deepest that brackets, `if`, `not` and unary minus may nest.
pub const MAX_NESTING: u32 = 32;

/// The longest source the parser accepts, in bytes. This is the size
/// limit of one value (05 §13.2).
pub const MAX_SOURCE_LEN: usize = 65_536;

/// Parses an expression.
///
/// ```
/// use clash_expr::{parse_expr, print_expr};
///
/// let tree = parse_expr("self.var.kills * 2 >= $limit").unwrap();
/// assert_eq!(print_expr(&tree), "self.var.kills * 2 >= $limit");
///
/// let error = parse_expr("1 + * 2").unwrap_err();
/// assert_eq!(error.span.start, 4);
/// ```
pub fn parse_expr(source: &str) -> Result<Expr, Diagnostic> {
    check_length(source)?;
    let mut parser = Parser::new(source);
    parser.start(0).map_err(|error| *error)?;
    if parser.peek.tok == Tok::End {
        return Err(Diagnostic::error(
            codes::EMPTY_EXPRESSION,
            parser.peek.span,
            "the expression is empty",
        ));
    }
    let tree = parser.expr().map_err(|error| *error)?;
    if parser.peek.tok != Tok::End {
        return Err(*parser.unexpected("an operator or the end of the expression"));
    }
    Ok(*tree.expr)
}

/// Parses a text template: literal text with expressions in braces.
///
/// `{{` and `}}` are literal braces. An expression may be followed by a
/// colon and a number format, `.N`, for N digits after the point.
/// Everything else, style tags included (12 §4.3), is literal text.
///
/// ```
/// use clash_expr::{TemplatePart, parse_template};
///
/// let template = parse_template("{the(self)} takes {event.energy:.0} J.").unwrap();
/// assert_eq!(template.parts.len(), 4);
/// assert!(matches!(&template.parts[1], TemplatePart::Text(text) if text == " takes "));
///
/// let errors = parse_template("{1 +} and {2 *}").unwrap_err();
/// assert_eq!(errors.len(), 2);
/// ```
pub fn parse_template(source: &str) -> Result<Template, Vec<Diagnostic>> {
    check_length(source).map_err(|error| vec![error])?;
    let bytes = source.as_bytes();
    let mut parts = Vec::new();
    let mut errors = Vec::new();
    let mut text = String::new();
    let mut parser = Parser::new(source);
    let mut at = 0;
    let mut run = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'{' if bytes.get(at + 1) == Some(&b'{') => {
                text.push_str(&source[run..at]);
                text.push('{');
                at += 2;
                run = at;
            }
            b'}' if bytes.get(at + 1) == Some(&b'}') => {
                text.push_str(&source[run..at]);
                text.push('}');
                at += 2;
                run = at;
            }
            b'}' => {
                errors.push(
                    Diagnostic::error(
                        codes::UNBALANCED_BRACE,
                        pos_span(at, at + 1),
                        "`}` with no `{` before it",
                    )
                    .with_suggestion("a literal brace is written twice: `}}`"),
                );
                at += 1;
            }
            b'{' => {
                text.push_str(&source[run..at]);
                if !text.is_empty() {
                    parts.push(TemplatePart::Text(core::mem::take(&mut text)));
                }
                match parser.template_expr(at) {
                    Ok((part, end)) => {
                        parts.push(part);
                        at = end;
                    }
                    Err(error) => {
                        errors.push(*error);
                        // Carry on after the next closing brace, so that
                        // each expression gets its own report.
                        at = source[at..].find('}').map_or(bytes.len(), |i| at + i + 1);
                    }
                }
                run = at;
            }
            _ => at += 1,
        }
    }
    text.push_str(&source[run..]);
    if !text.is_empty() {
        parts.push(TemplatePart::Text(text));
    }
    if errors.is_empty() {
        Ok(Template { parts })
    } else {
        Err(errors)
    }
}

fn check_length(source: &str) -> Result<(), Diagnostic> {
    if source.len() > MAX_SOURCE_LEN {
        return Err(Diagnostic::error(
            codes::TOO_DEEP,
            Span::at(0),
            format!(
                "the text is {} bytes long; the most allowed is {MAX_SOURCE_LEN}",
                source.len()
            ),
        ));
    }
    Ok(())
}

fn pos_span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

/// A failure. Boxed, as subtrees are, so that the parser's stack frames
/// stay small: the grammar is thirteen productions deep for every level
/// of brackets.
type Fail = Box<Diagnostic>;

/// A subtree and its height, which the parser tracks so that it can
/// refuse a tree that is too deep the moment it would be built.
struct Built {
    expr: Box<Expr>,
    height: u32,
}

type Parse = Result<Built, Fail>;

struct Parser<'s> {
    lexer: Lexer<'s>,
    /// The next token, not yet consumed.
    peek: Token,
    /// How deeply the parser has recursed.
    nesting: u32,
}

impl<'s> Parser<'s> {
    fn new(source: &'s str) -> Parser<'s> {
        Parser {
            lexer: Lexer::new(source),
            peek: Token {
                tok: Tok::End,
                span: Span::default(),
            },
            nesting: 0,
        }
    }

    /// Starts reading tokens at a byte position.
    fn start(&mut self, at: usize) -> Result<(), Fail> {
        self.nesting = 0;
        self.peek = self.lexer.next(at).map_err(Box::new)?;
        Ok(())
    }

    /// Consumes the next token, reads the one after it, and returns the
    /// span of the one consumed.
    fn bump(&mut self) -> Result<Span, Fail> {
        let span = self.peek.span;
        self.peek = self.lexer.next(span.end as usize).map_err(Box::new)?;
        Ok(span)
    }

    /// Consumes the next token and returns it whole.
    fn take(&mut self) -> Result<Token, Fail> {
        let next = self
            .lexer
            .next(self.peek.span.end as usize)
            .map_err(Box::new)?;
        Ok(core::mem::replace(&mut self.peek, next))
    }

    fn unexpected(&self, wanted: &str) -> Fail {
        let found = self.peek.tok.describe();
        let code = if self.peek.tok == Tok::End {
            codes::UNEXPECTED_END
        } else {
            codes::UNEXPECTED_TOKEN
        };
        Box::new(Diagnostic::error(
            code,
            self.peek.span,
            format!("expected {wanted}, found {found}"),
        ))
    }

    fn expect(&mut self, tok: &Tok, wanted: &str) -> Result<Span, Fail> {
        if &self.peek.tok == tok {
            self.bump()
        } else {
            Err(self.unexpected(wanted))
        }
    }

    fn enter(&mut self) -> Result<(), Fail> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return Err(Box::new(Diagnostic::error(
                codes::TOO_DEEP,
                self.peek.span,
                format!("the expression nests more than {MAX_NESTING} deep"),
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    fn node(&self, kind: ExprKind, span: Span, child_height: u32) -> Parse {
        let height = child_height + 1;
        if height > MAX_TREE_HEIGHT {
            return Err(Box::new(
                Diagnostic::error(
                    codes::TOO_DEEP,
                    span,
                    format!("the expression is more than {MAX_TREE_HEIGHT} operations deep"),
                )
                .with_suggestion("split it into functions or locals"),
            ));
        }
        Ok(Built {
            expr: Box::new(Expr::new(kind, span)),
            height,
        })
    }

    fn leaf(&self, kind: ExprKind, span: Span) -> Parse {
        self.node(kind, span, 0)
    }

    fn binary(&self, op: BinaryOp, lhs: Built, rhs: Built) -> Parse {
        let span = lhs.expr.span.to(rhs.expr.span);
        let height = lhs.height.max(rhs.height);
        self.node(
            ExprKind::Binary {
                op,
                lhs: lhs.expr,
                rhs: rhs.expr,
            },
            span,
            height,
        )
    }

    fn unary_node(&self, op: UnaryOp, start: Span, operand: Built) -> Parse {
        let span = start.to(operand.expr.span);
        self.node(
            ExprKind::Unary {
                op,
                operand: operand.expr,
            },
            span,
            operand.height,
        )
    }

    fn expr(&mut self) -> Parse {
        self.enter()?;
        let result = if self.peek.tok == Tok::If {
            self.if_expr()
        } else {
            self.or()
        };
        self.leave();
        result
    }

    fn if_expr(&mut self) -> Parse {
        let start = self.bump()?;
        let cond = self.expr()?;
        self.expect(&Tok::Then, "`then`")?;
        let then = self.expr()?;
        self.expect(&Tok::Else, "`else`")?;
        let otherwise = self.expr()?;
        let span = start.to(otherwise.expr.span);
        let height = cond.height.max(then.height).max(otherwise.height);
        self.node(
            ExprKind::If {
                cond: cond.expr,
                then: then.expr,
                otherwise: otherwise.expr,
            },
            span,
            height,
        )
    }

    fn or(&mut self) -> Parse {
        let mut lhs = self.and()?;
        while self.peek.tok == Tok::Or {
            self.bump()?;
            let rhs = self.and()?;
            lhs = self.binary(BinaryOp::Or, lhs, rhs)?;
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Parse {
        let mut lhs = self.not()?;
        while self.peek.tok == Tok::And {
            self.bump()?;
            let rhs = self.not()?;
            lhs = self.binary(BinaryOp::And, lhs, rhs)?;
        }
        Ok(lhs)
    }

    fn not(&mut self) -> Parse {
        if self.peek.tok != Tok::Not {
            return self.compare();
        }
        let start = self.bump()?;
        self.enter()?;
        let operand = self.not();
        self.leave();
        self.unary_node(UnaryOp::Not, start, operand?)
    }

    fn comparison_op(&self) -> Option<BinaryOp> {
        Some(match self.peek.tok {
            Tok::EqEq => BinaryOp::Eq,
            Tok::NotEq => BinaryOp::Ne,
            Tok::Lt => BinaryOp::Lt,
            Tok::Le => BinaryOp::Le,
            Tok::Gt => BinaryOp::Gt,
            Tok::Ge => BinaryOp::Ge,
            Tok::In => BinaryOp::In,
            _ => return None,
        })
    }

    fn compare(&mut self) -> Parse {
        let lhs = self.coalesce()?;
        let Some(op) = self.comparison_op() else {
            return Ok(lhs);
        };
        self.bump()?;
        let rhs = self.coalesce()?;
        if self.comparison_op().is_some() {
            return Err(Box::new(
                Diagnostic::error(
                    codes::CHAINED_COMPARISON,
                    self.peek.span,
                    "comparisons do not chain",
                )
                .with_suggestion("join two comparisons with `and`, or use brackets"),
            ));
        }
        self.binary(op, lhs, rhs)
    }

    fn coalesce(&mut self) -> Parse {
        let mut lhs = self.sum()?;
        while self.peek.tok == Tok::Coalesce {
            self.bump()?;
            let rhs = self.sum()?;
            lhs = self.binary(BinaryOp::Coalesce, lhs, rhs)?;
        }
        Ok(lhs)
    }

    fn sum(&mut self) -> Parse {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek.tok {
                Tok::Plus => BinaryOp::Add,
                Tok::Minus => BinaryOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump()?;
            let rhs = self.term()?;
            lhs = self.binary(op, lhs, rhs)?;
        }
    }

    fn term(&mut self) -> Parse {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek.tok {
                Tok::Star => BinaryOp::Mul,
                Tok::Slash => BinaryOp::Div,
                Tok::Percent => BinaryOp::Rem,
                _ => return Ok(lhs),
            };
            self.bump()?;
            let rhs = self.unary()?;
            lhs = self.binary(op, lhs, rhs)?;
        }
    }

    fn unary(&mut self) -> Parse {
        if self.peek.tok != Tok::Minus {
            return self.power();
        }
        let start = self.bump()?;
        self.enter()?;
        let operand = self.unary();
        self.leave();
        self.unary_node(UnaryOp::Neg, start, operand?)
    }

    fn power(&mut self) -> Parse {
        let base = self.postfix()?;
        if self.peek.tok != Tok::Caret {
            return Ok(base);
        }
        self.bump()?;
        self.enter()?;
        let exponent = self.unary();
        self.leave();
        self.binary(BinaryOp::Pow, base, exponent?)
    }

    fn postfix(&mut self) -> Parse {
        let mut object = self.primary()?;
        loop {
            object = match self.peek.tok {
                Tok::Dot => self.member(object)?,
                Tok::LParen => self.call(object)?,
                Tok::LBracket => self.index(object)?,
                _ => return Ok(object),
            };
        }
    }

    fn member(&mut self, object: Built) -> Parse {
        self.bump()?;
        let (name, name_span) = self.name("a property name after `.`")?;
        let span = object.expr.span.to(name_span);
        self.node(
            ExprKind::Member {
                object: object.expr,
                name,
            },
            span,
            object.height,
        )
    }

    fn call(&mut self, callee: Built) -> Parse {
        self.bump()?;
        let (args, height) = self.args(&Tok::RParen)?;
        let close = self.expect(&Tok::RParen, "`,` or `)`")?;
        let span = callee.expr.span.to(close);
        self.node(
            ExprKind::Call {
                callee: callee.expr,
                args,
            },
            span,
            callee.height.max(height),
        )
    }

    fn index(&mut self, object: Built) -> Parse {
        self.bump()?;
        let index = self.expr()?;
        let close = self.expect(&Tok::RBracket, "`]`")?;
        let span = object.expr.span.to(close);
        self.node(
            ExprKind::Index {
                object: object.expr,
                index: index.expr,
            },
            span,
            object.height.max(index.height),
        )
    }

    /// A comma-separated list of expressions, up to but not including the
    /// closing token. Returns the expressions and the height of the
    /// tallest.
    fn args(&mut self, close: &Tok) -> Result<(Vec<Expr>, u32), Fail> {
        let mut args = Vec::new();
        let mut height = 0;
        if &self.peek.tok == close {
            return Ok((args, height));
        }
        loop {
            let arg = self.expr()?;
            height = height.max(arg.height);
            args.push(*arg.expr);
            if self.peek.tok != Tok::Comma {
                return Ok((args, height));
            }
            self.bump()?;
        }
    }

    fn name(&mut self, wanted: &str) -> Result<(Name, Span), Fail> {
        if !matches!(self.peek.tok, Tok::Name(_)) {
            return Err(self.unexpected(wanted));
        }
        let token = self.take()?;
        match token.tok {
            Tok::Name(name) => Ok((name, token.span)),
            _ => Err(self.unexpected(wanted)),
        }
    }

    fn primary(&mut self) -> Parse {
        match self.peek.tok {
            Tok::Number(_) => self.number(),
            Tok::Str(_) => self.string(),
            Tok::True => {
                let span = self.bump()?;
                self.leaf(ExprKind::Bool(true), span)
            }
            Tok::False => {
                let span = self.bump()?;
                self.leaf(ExprKind::Bool(false), span)
            }
            Tok::Null => {
                let span = self.bump()?;
                self.leaf(ExprKind::Null, span)
            }
            Tok::Name(_) => {
                let (name, span) = self.name("a name")?;
                self.leaf(ExprKind::Name(name), span)
            }
            Tok::Dollar => self.local(),
            Tok::LParen => self.bracketed(),
            Tok::LBracket => self.list(),
            Tok::LBrace => self.map(),
            _ => Err(self.unexpected("a value")),
        }
    }

    fn string(&mut self) -> Parse {
        let token = self.take()?;
        match token.tok {
            Tok::Str(text) => self.leaf(ExprKind::Str(text), token.span),
            _ => Err(self.unexpected("a value")),
        }
    }

    fn local(&mut self) -> Parse {
        let dollar = self.bump()?;
        if self.peek.span.start != dollar.end {
            return Err(self.unexpected("a name directly after `$`"));
        }
        let (name, span) = self.name("a name after `$`")?;
        self.leaf(ExprKind::Local(name), dollar.to(span))
    }

    fn bracketed(&mut self) -> Parse {
        let open = self.bump()?;
        let mut inner = self.expr()?;
        let close = self.expect(&Tok::RParen, "`)`")?;
        // Brackets leave no node; the tree's span grows to cover them so
        // that a diagnostic underlines what was written.
        inner.expr.span = open.to(close);
        Ok(inner)
    }

    fn list(&mut self) -> Parse {
        let open = self.bump()?;
        let (items, height) = self.args(&Tok::RBracket)?;
        let close = self.expect(&Tok::RBracket, "`,` or `]`")?;
        self.node(ExprKind::List(items), open.to(close), height)
    }

    fn number(&mut self) -> Parse {
        let token = self.take()?;
        let Tok::Number(value) = token.tok else {
            return Err(self.unexpected("a number"));
        };
        let mut span = token.span;
        let mut unit = None;
        let symbol = match &self.peek.tok {
            // Letters written against the number are always a unit.
            Tok::Unit(symbol) => Some(symbol.as_str()),
            // A name after a space is a unit only if it is one; anything
            // else is left to be reported as a missing operator.
            Tok::Name(name) if !name.is_qualified() && Unit::from_symbol(&name.name).is_some() => {
                Some(name.name.as_str())
            }
            _ => None,
        };
        if let Some(symbol) = symbol {
            let unit_span = self.peek.span;
            match Unit::from_symbol(symbol) {
                Some(found) => {
                    self.bump()?;
                    span = span.to(unit_span);
                    unit = Some(found);
                }
                None => return Err(Box::new(unknown_unit(symbol, unit_span))),
            }
        }
        self.leaf(ExprKind::Number { value, unit }, span)
    }

    fn map(&mut self) -> Parse {
        let open = self.bump()?;
        let mut entries = Vec::new();
        let mut height = 0;
        if self.peek.tok != Tok::RBrace {
            loop {
                let key = self.expr()?;
                self.expect(&Tok::Colon, "`:` after a map key")?;
                let value = self.expr()?;
                height = height.max(key.height).max(value.height);
                entries.push((*key.expr, *value.expr));
                if self.peek.tok != Tok::Comma {
                    break;
                }
                self.bump()?;
            }
        }
        let close = self.expect(&Tok::RBrace, "`,` or `}`")?;
        self.node(ExprKind::Map(entries), open.to(close), height)
    }

    /// The expression of a text template that opens with the brace at
    /// byte `open`. Returns the part and the position after its closing
    /// brace.
    fn template_expr(&mut self, open: usize) -> Result<(TemplatePart, usize), Fail> {
        let source = self.lexer.source();
        let unclosed = || {
            Box::new(
                Diagnostic::error(
                    codes::UNBALANCED_BRACE,
                    pos_span(open, open + 1),
                    "`{` with no `}` after it",
                )
                .with_suggestion("a literal brace is written twice: `{{`"),
            )
        };
        self.start(open + 1)?;
        match self.peek.tok {
            Tok::End => return Err(unclosed()),
            Tok::RBrace => {
                return Err(Box::new(Diagnostic::error(
                    codes::EMPTY_EXPRESSION,
                    pos_span(open, self.peek.span.end as usize),
                    "there is no expression between the braces",
                )));
            }
            _ => {}
        }
        let tree = self.expr()?;
        let mut format = None;
        let end = match self.peek.tok {
            Tok::RBrace => self.peek.span.end as usize,
            Tok::Colon => {
                let start = self.peek.span.end as usize;
                let Some(len) = source[start..].find('}') else {
                    return Err(unclosed());
                };
                let spec = &source[start..start + len];
                let spec_span = pos_span(start, start + len);
                format = Some(parse_format(spec, spec_span).map_err(Box::new)?);
                start + len + 1
            }
            Tok::End => return Err(unclosed()),
            _ => return Err(self.unexpected("an operator, `:` or `}`")),
        };
        Ok((
            TemplatePart::Expr {
                expr: *tree.expr,
                format,
                span: pos_span(open, end),
            },
            end,
        ))
    }
}

fn unknown_unit(symbol: &str, span: Span) -> Diagnostic {
    let error = Diagnostic::error(
        codes::UNKNOWN_UNIT,
        span,
        format!("unknown unit `{symbol}`"),
    );
    if symbol == "C" {
        return error.with_suggestion(
            "`C` is an offset and not a scale; write kelvin, or use a JSON unit string",
        );
    }
    let all: Vec<&str> = Unit::all().map(Unit::symbol).collect();
    match crate::diag::closest_name(symbol, all.iter().copied()) {
        Some(close) => error.with_suggestion(format!("did you mean `{close}`?")),
        None => error.with_suggestion(format!("the units are: {}", all.join(" "))),
    }
}

/// A number format: `.N`.
fn parse_format(spec: &str, span: Span) -> Result<Format, Diagnostic> {
    let bad = || {
        Diagnostic::error(
            codes::BAD_FORMAT,
            span,
            format!("`{spec}` is not a number format"),
        )
        .with_suggestion(format!(
            "write `.N` for N digits after the point, N from 0 to {MAX_FORMAT_PRECISION}"
        ))
    };
    let digits = spec.trim().strip_prefix('.').ok_or_else(bad)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad());
    }
    match digits.parse::<u8>() {
        Ok(precision) if precision <= MAX_FORMAT_PRECISION => Ok(Format { precision }),
        _ => Err(bad()),
    }
}
