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

/// 解析逗号分隔的搜索语句；引号内的逗号属于词条，空词条被忽略。
///
/// 双引号或末尾的 `$` 表示精确匹配。无命名空间的词条始终匹配
/// 标签名子串或标题子串。引号必须包住整个词条，不支持转义语法。
/// 通配符规则优先于精确标记：单个末尾 `*`/`%` 转为前缀；其他位置的
/// 通配符把词条切成几段，每段按子串匹配，并标记为已降级。
/// `pages`、`read` 的整数条件以及任意命名空间的比较运算均不支持。
pub fn parse_query(input: &str) -> Result<ParsedQuery, ParseError> {
    let mut parsed = ParsedQuery {
        query: Query::default(),
        degraded: false,
    };
    let mut quoted = false;
    let terms = input.split(|ch| match ch {
        '"' => {
            quoted = !quoted;
            false
        }
        ',' => !quoted,
        _ => false,
    });
    for raw in terms.map(str::trim).filter(|raw| !raw.is_empty()) {
        let (excluded, text) = match raw.strip_prefix('-') {
            Some(text) => (true, text.trim()),
            None => (false, raw),
        };
        let (mut exact, text) = match text.strip_suffix('$') {
            Some(text) => (true, text.trim_end()),
            None => (false, text),
        };
        let text = match text
            .strip_prefix('"')
            .and_then(|text| text.strip_suffix('"'))
        {
            Some(text) => {
                exact = true;
                text.trim()
            }
            None => text,
        };
        if text.is_empty() || text.contains('"') {
            return Err(ParseError::InvalidTerm(raw.into()));
        }
        if let Some((namespace, value)) = text.split_once(':') {
            let value = value.trim_start();
            let digits = value.trim_start_matches(['+', '-']);
            let integer = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
            if ((namespace.eq_ignore_ascii_case("pages") || namespace.eq_ignore_ascii_case("read"))
                && integer)
                || value.starts_with(['<', '>', '='])
                || value.starts_with("!=")
            {
                return Err(ParseError::UnsupportedNumericCondition(raw.into()));
            }
        }

        let (text, trailing_wildcard) = match text.strip_suffix(['*', '%']) {
            Some(text) => (text, true),
            None => (text, false),
        };
        let namespaced = text.contains(':');
        let substring = |fragment: &str| {
            if namespaced {
                Term::TagContains(fragment.into())
            } else {
                Term::Any(vec![
                    Term::TagContains(fragment.into()),
                    Term::TitleContains(fragment.into()),
                ])
            }
        };
        if text.contains(WILDCARDS) {
            // 在通配符处切开，要求每一段都出现。结果是真实匹配的超集，
            // 只适用于包含条件；排除超集会误删本该保留的归档。
            if excluded {
                return Err(ParseError::UnsupportedWildcard(raw.into()));
            }
            parsed.degraded = true;
            let fragments = text
                .split(WILDCARDS)
                .filter(|fragment| !fragment.is_empty());
            parsed.query.include.extend(fragments.map(substring));
            continue;
        }

        let term = if !namespaced {
            substring(text)
        } else if exact && !trailing_wildcard {
            Term::Tag(text.into())
        } else {
            Term::TagPrefix(text.into())
        };
        if excluded {
            parsed.query.exclude.push(term);
        } else {
            parsed.query.include.push(term);
        }
    }
    Ok(parsed)
}
