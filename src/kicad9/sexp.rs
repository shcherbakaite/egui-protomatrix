//! Minimal s-expression parser for KiCad .kicad_mod files.

use super::error::{Error, Result};
use std::iter::Peekable;
use std::str::Chars;

/// S-expression node: list or atom.
#[derive(Debug, Clone)]
pub enum Sexp {
    List(String, Vec<Sexp>),
    Atom(String),
}

impl Sexp {
    pub fn list_name(&self) -> Option<&str> {
        match self {
            Sexp::List(name, _) => Some(name),
            Sexp::Atom(_) => None,
        }
    }

    pub fn list_rest(&self) -> Option<&[Sexp]> {
        match self {
            Sexp::List(_, rest) => Some(rest),
            Sexp::Atom(_) => None,
        }
    }

    pub fn find(&self, name: &str) -> Option<&Sexp> {
        match self {
            Sexp::List(_, rest) => rest.iter().find(|e| e.list_name() == Some(name)),
            Sexp::Atom(_) => None,
        }
    }

    pub fn find_all<'a>(&'a self, name: &'a str) -> Box<dyn Iterator<Item = &Sexp> + 'a> {
        match self {
            Sexp::List(_, rest) => Box::new(rest.iter().filter(move |e| e.list_name() == Some(name))),
            Sexp::Atom(_) => Box::new(std::iter::empty()),
        }
    }

    pub fn string(&self) -> Option<&str> {
        match self {
            Sexp::Atom(s) => Some(s.as_str()),
            Sexp::List(_, _) => None,
        }
    }

    pub fn float(&self) -> Option<f64> {
        self.string()?.parse().ok()
    }

    pub fn as_list(&self) -> Option<&[Sexp]> {
        self.list_rest()
    }
}

/// Parse s-expression from string.
pub fn parse(input: &str) -> Result<Sexp> {
    let mut p = Parser::new(input);
    p.parse_sexp().and_then(|s| {
        p.skip_whitespace();
        if p.peek().is_none() {
            Ok(s)
        } else {
            Err(Error::Parse {
                message: "Trailing content after s-expression".into(),
                offset: Some(p.offset()),
            })
        }
    })
}

struct Parser<'a> {
    chars: Peekable<Chars<'a>>,
    offset: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser {
            chars: input.chars().peekable(),
            offset: 0,
        }
    }

    fn offset(&self) -> usize {
        self.offset
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.offset += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace() || c == '\n' || c == '\r') {
            self.advance();
        }
    }

    fn parse_sexp(&mut self) -> Result<Sexp> {
        self.skip_whitespace();
        match self.peek() {
            Some('(') => self.parse_list(),
            Some('"') => self.parse_quoted_string(),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '.' => self.parse_number(),
            Some(c) if c.is_alphanumeric() || c == '_' || c == '*' || c == '.' => {
                self.parse_atom()
            }
            Some(c) => Err(Error::Parse {
                message: format!("Unexpected character '{}'", c),
                offset: Some(self.offset()),
            }),
            None => Err(Error::Parse {
                message: "Unexpected end of input".into(),
                offset: Some(self.offset()),
            }),
        }
    }

    fn parse_list(&mut self) -> Result<Sexp> {
        let _open = self.advance().ok_or_else(|| Error::Parse {
            message: "Expected '('".into(),
            offset: Some(self.offset()),
        })?;
        self.skip_whitespace();

        let name = match self.peek() {
            Some('(') | None => {
                return Err(Error::Parse {
                    message: "Empty list or nested list as list name".into(),
                    offset: Some(self.offset()),
                });
            }
            Some('"') => self.parse_quoted_string()?.string().unwrap_or("").to_string(),
            Some(_) => self.parse_atom()?.string().unwrap_or("").to_string(),
        };

        let mut rest = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(')') => {
                    self.advance();
                    break;
                }
                None => {
                    return Err(Error::Parse {
                        message: "Unclosed list".into(),
                        offset: Some(self.offset()),
                    });
                }
                _ => {
                    rest.push(self.parse_sexp()?);
                }
            }
        }
        Ok(Sexp::List(name, rest))
    }

    fn parse_quoted_string(&mut self) -> Result<Sexp> {
        let _quote = self.advance().ok_or_else(|| Error::Parse {
            message: "Expected '\"'".into(),
            offset: Some(self.offset()),
        })?;

        let mut s = String::new();
        loop {
            match self.advance() {
                None => {
                    return Err(Error::Parse {
                        message: "Unclosed string".into(),
                        offset: Some(self.offset()),
                    });
                }
                Some('"') => break,
                Some('\\') => {
                    if let Some(c) = self.advance() {
                        s.push(c);
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Sexp::Atom(s))
    }

    fn parse_number(&mut self) -> Result<Sexp> {
        let mut s = String::new();
        if self.peek() == Some('-') {
            s.push(self.advance().unwrap());
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            s.push(self.advance().unwrap());
        }
        if s.is_empty() {
            return Err(Error::Parse {
                message: "Expected number".into(),
                offset: Some(self.offset()),
            });
        }
        Ok(Sexp::Atom(s))
    }

    fn parse_atom(&mut self) -> Result<Sexp> {
        let mut s = String::new();
        while matches!(
            self.peek(),
            Some(c) if c.is_alphanumeric() || c == '_' || c == '*' || c == '.' || c == ':' || c == '-'
        ) {
            s.push(self.advance().unwrap());
        }
        if s.is_empty() {
            return Err(Error::Parse {
                message: "Expected atom".into(),
                offset: Some(self.offset()),
            });
        }
        Ok(Sexp::Atom(s))
    }
}
