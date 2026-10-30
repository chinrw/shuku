use crate::{Query, Term};

/// 解析后的搜索条件沿用 [`Query`] 的默认排序和分页参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub query: Query,
    /// 至少一个词条中的通配符已降级为子串匹配。
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("无效的搜索词条：{0}")]
    InvalidTerm(String),
    #[error("不支持的数值搜索条件：{0}")]
    UnsupportedNumericCondition(String),
    #[error("排除条件暂不支持中间位置的通配符：{0}")]
    UnsupportedWildcard(String),
}

/// LRR 的通配符：`*`、`%` 匹配任意多个字符，`?`、`_` 匹配单个字符。
const WILDCARDS: [char; 4] = ['*', '%', '?', '_'];

/// 词条里的一个字符。`escaped` 为真表示它前面有反斜杠，只当普通字符，
/// 不再充当引号、逗号、排除标记、精确标记或通配符。
#[derive(Debug, Clone, Copy)]
struct Piece {
    ch: char,
    escaped: bool,
}

impl Piece {
    fn is(self, ch: char) -> bool {
        !self.escaped && self.ch == ch
    }

    fn is_wildcard(self) -> bool {
        !self.escaped && WILDCARDS.contains(&self.ch)
    }
}

/// 解析逗号分隔的搜索语句；引号内的逗号属于词条，空词条被忽略。
///
/// 双引号或末尾的 `$` 表示精确匹配。无命名空间的词条始终匹配
/// 标签名子串或标题子串。引号必须包住整个词条。反斜杠让下一个字符
/// 失去语法含义（`\"`、`\,`、`\-`、`\$`、`\*` 等），`\\` 是一个反斜杠，
/// 词条末尾孤立的反斜杠按字面保留；`:` 无论是否转义都是命名空间分隔符。
/// 通配符规则优先于精确标记：单个末尾 `*`/`%` 转为前缀；其他位置的
/// 通配符把词条切成几段，每段按子串匹配，并标记为已降级。
/// `pages`、`read` 的整数条件以及任意命名空间的比较运算均不支持。
pub fn parse_query(input: &str) -> Result<ParsedQuery, ParseError> {
    let mut parsed = ParsedQuery {
        query: Query::default(),
        degraded: false,
    };
    let (mut quoted, mut escaped) = (false, false);
    let terms = input.split(|ch| {
        if std::mem::take(&mut escaped) {
            return false;
        }
        match ch {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            ',' => return !quoted,
            _ => {}
        }
        false
    });
    for raw in terms {
        parse_term(raw, &mut parsed)?;
    }
    Ok(parsed)
}

/// 去掉反斜杠，把词条解码成带转义标记的字符序列。
fn decode(raw: &str) -> Vec<Piece> {
    let mut chars = raw.chars();
    let mut pieces = Vec::with_capacity(raw.len());
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            pieces.push(Piece { ch, escaped: false });
            continue;
        }
        // 反斜杠后面没有字符时按字面保留，不报错。
        let ch = chars.next().unwrap_or('\\');
        pieces.push(Piece { ch, escaped: true });
    }
    pieces
}

/// 去掉首尾未转义的空白。
fn trim(pieces: &[Piece]) -> &[Piece] {
    let blank = |piece: &Piece| !piece.escaped && piece.ch.is_whitespace();
    let start = pieces
        .iter()
        .position(|p| !blank(p))
        .unwrap_or(pieces.len());
    let end = pieces
        .iter()
        .rposition(|p| !blank(p))
        .map_or(start, |i| i + 1);
    &pieces[start..end]
}

fn text(pieces: &[Piece]) -> String {
    pieces.iter().map(|piece| piece.ch).collect()
}

/// `pages:>20`、`read:0` 这类数值条件按原文判断，转义不影响它。
fn is_numeric_condition(text: &str) -> bool {
    let Some((namespace, value)) = text.split_once(':') else {
        return false;
    };
    let value = value.trim_start();
    let digits = value.trim_start_matches(['+', '-']);
    let integer = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    ((namespace.eq_ignore_ascii_case("pages") || namespace.eq_ignore_ascii_case("read")) && integer)
        || value.starts_with(['<', '>', '='])
        || value.starts_with("!=")
}

/// `raw` 是未经修剪的原文：先解码再去空白，转义的空白（`foo\ `）才能留在词条里。
/// 错误里带的是修剪过的原文。
fn parse_term(raw: &str, parsed: &mut ParsedQuery) -> Result<(), ParseError> {
    let pieces = decode(raw);
    let mut pieces = trim(pieces.as_slice());
    if pieces.is_empty() {
        return Ok(());
    }
    let raw = raw.trim();
    let excluded = match pieces.split_first() {
        Some((first, rest)) if first.is('-') => {
            pieces = trim(rest);
            true
        }
        _ => false,
    };
    let mut exact = match pieces.split_last() {
        Some((last, rest)) if last.is('$') => {
            pieces = trim(rest);
            true
        }
        _ => false,
    };
    if pieces.len() >= 2 && pieces[0].is('"') && pieces[pieces.len() - 1].is('"') {
        exact = true;
        pieces = trim(&pieces[1..pieces.len() - 1]);
    }
    if pieces.is_empty() || pieces.iter().any(|piece| piece.is('"')) {
        return Err(ParseError::InvalidTerm(raw.into()));
    }
    if is_numeric_condition(&text(pieces)) {
        return Err(ParseError::UnsupportedNumericCondition(raw.into()));
    }

    let trailing_wildcard = pieces
        .last()
        .is_some_and(|last| last.is('*') || last.is('%'));
    if trailing_wildcard {
        pieces = &pieces[..pieces.len() - 1];
    }
    let namespaced = pieces.iter().any(|piece| piece.ch == ':');
    let substring = |fragment: String| {
        if namespaced {
            Term::TagContains(fragment)
        } else {
            Term::Any(vec![
                Term::TagContains(fragment.clone()),
                Term::TitleContains(fragment),
            ])
        }
    };
    if pieces.iter().any(|piece| piece.is_wildcard()) {
        // 在通配符处切开，要求每一段都出现。结果是真实匹配的超集，
        // 只适用于包含条件；排除超集会误删本该保留的归档。
        if excluded {
            return Err(ParseError::UnsupportedWildcard(raw.into()));
        }
        parsed.degraded = true;
        let fragments = pieces
            .split(|piece| piece.is_wildcard())
            .filter(|fragment| !fragment.is_empty())
            .map(text);
        parsed.query.include.extend(fragments.map(substring));
        return Ok(());
    }

    let text = text(pieces);
    let term = if !namespaced {
        substring(text)
    } else if exact && !trailing_wildcard {
        Term::Tag(text)
    } else {
        Term::TagPrefix(text)
    };
    if excluded {
        parsed.query.exclude.push(term);
    } else {
        parsed.query.include.push(term);
    }
    Ok(())
}
