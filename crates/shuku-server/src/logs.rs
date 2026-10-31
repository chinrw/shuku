//! 进程内最近的日志，控制台的日志页从这里读。只是一个环形缓冲区，重启后清空。

use std::{
    collections::VecDeque,
    fmt::Write,
    sync::{Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context};

/// 最多保留这么多条，旧的先丢。
const CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
pub(crate) struct LogEntry {
    /// 递增的序号，客户端用它只取新的日志。
    pub(crate) seq: u64,
    /// Unix 毫秒。
    pub(crate) time: i64,
    pub(crate) level: Level,
    pub(crate) message: String,
}

struct Buffer {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
}

static BUFFER: Mutex<Buffer> = Mutex::new(Buffer {
    entries: VecDeque::new(),
    next_seq: 1,
});

/// 把日志事件记进缓冲区的 tracing 层，由可执行文件装进全局订阅者。
pub struct LogLayer;

pub fn log_layer() -> LogLayer {
    LogLayer
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut message = Message::default();
        event.record(&mut message);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        let mut buffer = BUFFER.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = buffer.next_seq;
        buffer.next_seq += 1;
        if buffer.entries.len() == CAPACITY {
            buffer.entries.pop_front();
        }
        buffer.entries.push_back(LogEntry {
            seq,
            time,
            level: *event.metadata().level(),
            message: message.text,
        });
    }
}

/// 和终端里的格式一样：先是消息，后面跟 `字段=值`。
#[derive(Default)]
struct Message {
    text: String,
}

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let fields = std::mem::take(&mut self.text);
            let _ = write!(self.text, "{value:?}");
            self.text.push_str(&fields);
        } else {
            let _ = write!(self.text, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.text.insert_str(0, value);
        } else {
            let _ = write!(self.text, " {}={value}", field.name());
        }
    }
}

/// 序号大于 `after`、级别不低于 `level` 的日志，最多 `limit` 条，取最新的那些，按时间顺序。
pub(crate) fn recent(level: Level, after: u64, limit: usize) -> Vec<LogEntry> {
    let buffer = BUFFER.lock().unwrap_or_else(PoisonError::into_inner);
    // tracing 的 Level 越详细越“大”：ERROR < WARN < INFO。
    let mut entries: Vec<LogEntry> = buffer
        .entries
        .iter()
        .rev()
        .filter(|entry| entry.seq > after && entry.level <= level)
        .take(limit)
        .cloned()
        .collect();
    entries.reverse();
    entries
}
