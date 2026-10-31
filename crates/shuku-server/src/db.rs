//! 唯一的写连接、只读连接池，以及把进度上报和指纹补算合并提交的写队列。

use crate::{Error, Result};
use shuku_store::{PooledReader, Progress, ReaderPool, ScanEntry, Store, Writer};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, oneshot};

/// 两次写队列提交之间至少隔这么久。空闲后的第一条请求立即提交，之后到达的攒到下一次。
/// 单本一个事务时，WAL 提交在 btrfs 上的 p99 有 20 到 30 ms，合并后每秒只付一次。
const COMMIT_INTERVAL: Duration = Duration::from_secs(1);
/// 写队列满了以后发送方阻塞，补算指纹不会把未提交的请求无限堆在内存里。
const QUEUE_CAPACITY: usize = 1024;
/// 没有写入这么久以后才做 checkpoint，免得和写入抢同一把锁。
const CHECKPOINT_IDLE: Duration = Duration::from_secs(2);
/// 一直有写入时（例如扫描整个书库），最多隔这么久也做一次，不让 WAL 无限变长。
const CHECKPOINT_MAX_DELAY: Duration = Duration::from_secs(30);
/// 写队列线程没有请求时，隔这么久醒来检查一次要不要 checkpoint。
const IDLE_POLL: Duration = Duration::from_secs(1);

pub(crate) struct Database {
    /// 唯一的写连接。读走 `readers`，不和写争这把锁。
    store: Mutex<Store>,
    readers: ReaderPool,
    wal: Mutex<Wal>,
}

/// WAL 里有没有还没写回数据库文件的提交，以及最近一次写入和 checkpoint 的时间。
struct Wal {
    dirty: bool,
    last_write: Instant,
    last_checkpoint: Instant,
}

impl Database {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let store = Store::open(path)?;
        store.disable_auto_checkpoint()?;
        let now = Instant::now();
        Ok(Self {
            store: Mutex::new(store),
            readers: ReaderPool::new(path),
            wal: Mutex::new(Wal {
                dirty: false,
                last_write: now,
                last_checkpoint: now,
            }),
        })
    }

    pub(crate) fn reader(&self) -> Result<PooledReader<'_>> {
        Ok(self.readers.get()?)
    }

    /// 在写连接上执行一个事务。所有写入都经过这里，checkpoint 才知道什么时候空闲。
    pub(crate) fn write<T>(&self, work: impl FnOnce(&mut Writer<'_>) -> Result<T>) -> Result<T> {
        let mut store = self.store.lock().map_err(|_| Error::Lock)?;
        let value = store.write(work)?;
        let mut wal = self.wal.lock().map_err(|_| Error::Lock)?;
        wal.dirty = true;
        wal.last_write = Instant::now();
        Ok(value)
    }

    /// 空闲时或者拖得太久时做一次 PASSIVE checkpoint。
    fn checkpoint_if_due(&self) -> Result<()> {
        if !self.wal.lock().map_err(|_| Error::Lock)?.is_due() {
            return Ok(());
        }
        // 和 `write` 一样先拿写连接再拿 `wal`，持有写连接时状态不会再变。
        let store = self.store.lock().map_err(|_| Error::Lock)?;
        let mut wal = self.wal.lock().map_err(|_| Error::Lock)?;
        if !wal.is_due() {
            return Ok(());
        }
        // 还有读者在用旧快照时只能写回一部分，剩下的保持 dirty，下次再试。
        wal.dirty = !store.checkpoint()?;
        wal.last_checkpoint = Instant::now();
        Ok(())
    }
}

impl Wal {
    fn is_due(&self) -> bool {
        self.dirty
            && (self.last_write.elapsed() >= CHECKPOINT_IDLE
                || self.last_checkpoint.elapsed() >= CHECKPOINT_MAX_DELAY)
    }
}

pub(crate) enum Request {
    Progress(ProgressRequest),
    ContentHash(HashRequest),
}

pub(crate) struct ProgressRequest {
    pub(crate) id: i64,
    pub(crate) page: u32,
    pub(crate) client_time: i64,
    /// 所在的事务提交后才回复，收到回复的上报一定已经写进数据库。
    pub(crate) reply: oneshot::Sender<Result<ProgressReply>>,
}

pub(crate) struct ProgressReply {
    pub(crate) accepted: bool,
    pub(crate) progress: Option<Progress>,
}

pub(crate) struct HashRequest {
    pub(crate) entry: ScanEntry,
    pub(crate) hash: String,
    /// 回复是否写入。补算线程不等回复，继续算下一本。
    pub(crate) reply: mpsc::Sender<Result<bool>>,
}

/// 启动写队列线程。所有发送端都丢掉后，线程提交剩下的请求后退出。
/// 提交了被接受的进度时通知 `progress_changed`。
pub(crate) fn spawn_writer(
    database: Arc<Database>,
    progress_changed: Arc<Notify>,
) -> Result<SyncSender<Request>> {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("shuku-writer".into())
        .spawn(move || run_writer(&database, &receiver, &progress_changed))?;
    Ok(sender)
}

pub(crate) fn queue_stopped() -> Error {
    Error::Write("写队列已停止".into())
}

fn run_writer(database: &Database, receiver: &Receiver<Request>, progress_changed: &Notify) {
    let mut pending = Vec::new();
    let mut last_commit: Option<Instant> = None;
    loop {
        let due = last_commit.map_or(Duration::ZERO, |last| {
            COMMIT_INTERVAL.saturating_sub(last.elapsed())
        });
        let wait = if pending.is_empty() { IDLE_POLL } else { due };
        match receiver.recv_timeout(wait) {
            Ok(request) => pending.push(request),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                commit(database, std::mem::take(&mut pending), progress_changed);
                return;
            }
        }
        pending.extend(receiver.try_iter());
        if !pending.is_empty() && last_commit.is_none_or(|last| last.elapsed() >= COMMIT_INTERVAL) {
            commit(database, std::mem::take(&mut pending), progress_changed);
            last_commit = Some(Instant::now());
        }
        if let Err(error) = database.checkpoint_if_due() {
            tracing::error!(%error, "checkpoint 失败");
        }
    }
}

/// 一批请求在一个事务里提交，每条请求各占一个保存点，单条出错不影响其他请求。
fn commit(database: &Database, requests: Vec<Request>, progress_changed: &Notify) {
    if requests.is_empty() {
        return;
    }
    let (mut progress, mut hashes) = (Vec::new(), Vec::new());
    for request in requests {
        match request {
            Request::Progress(request) => progress.push(request),
            Request::ContentHash(request) => hashes.push(request),
        }
    }
    // 单条请求的错误留在内层，只回给它自己；外层的 `?` 表示事务已经失效，整批失败。
    let outcomes = database.write(|writer| {
        let progress = progress
            .iter()
            .map(|request| writer.savepoint(|writer| write_progress(writer, request)))
            .collect::<shuku_store::Result<Vec<_>>>()?;
        let hashes = hashes
            .iter()
            .map(|request| {
                writer
                    .savepoint(|writer| Ok(writer.set_content_hash(&request.entry, &request.hash)?))
            })
            .collect::<shuku_store::Result<Vec<_>>>()?;
        Ok((progress, hashes))
    });
    let (progress_outcomes, hash_outcomes) = match outcomes {
        Ok(outcomes) => outcomes,
        Err(error) => {
            tracing::error!(%error, "写队列提交失败");
            let message = error.to_string();
            (
                progress
                    .iter()
                    .map(|_| Err(Error::Write(message.clone())))
                    .collect(),
                hashes
                    .iter()
                    .map(|_| Err(Error::Write(message.clone())))
                    .collect(),
            )
        }
    };
    if progress_outcomes
        .iter()
        .any(|outcome| outcome.as_ref().is_ok_and(|reply| reply.accepted))
    {
        progress_changed.notify_one();
    }
    // 请求方已经不等了（例如 HTTP 连接断开）时发送失败，写入照样有效。
    for (request, outcome) in progress.into_iter().zip(progress_outcomes) {
        let _ = request.reply.send(outcome);
    }
    for (request, outcome) in hashes.into_iter().zip(hash_outcomes) {
        let _ = request.reply.send(outcome);
    }
}

fn write_progress(writer: &mut Writer<'_>, request: &ProgressRequest) -> Result<ProgressReply> {
    let archive = writer.archive(request.id)?.ok_or_else(Error::not_found)?;
    if request.page >= archive.page_count {
        return Err(Error::bad_request("page 超出归档页数"));
    }
    let accepted = writer.write_progress(request.id, request.page, request.client_time)?;
    let progress = writer
        .archive(request.id)?
        .ok_or_else(Error::not_found)?
        .progress;
    Ok(ProgressReply { accepted, progress })
}
