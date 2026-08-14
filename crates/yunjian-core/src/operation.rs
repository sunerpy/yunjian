//! 全工作区唯一的长任务事件、取消与资源释放协议。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, WaitTimeoutResult};
use std::time::{Duration, Instant};

/// 普通事件队列最多容纳的事件数。
pub const EVENT_QUEUE_CAPACITY: usize = 256;

/// 长任务产生的有序事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Event<P, I> {
    /// 可合并的进度快照。
    Progress(P),
    /// 不可丢弃的增量结果。
    Item(I),
    /// 任务成功完成。
    Done,
    /// 任务响应取消后终止。
    Cancelled,
    /// 任务失败或生产者异常退出。
    Failed {
        /// 已脱敏、可展示的失败原因。
        message: String,
    },
}

impl<P, I> Event<P, I> {
    /// 该事件是否结束事件流。
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed { .. })
    }
}

/// 消费长任务事件并控制其生命周期的句柄。
#[derive(Debug)]
pub struct OperationHandle<P, I> {
    state: Arc<State<P, I>>,
}

/// 生产者用于发送事件和观察取消的入口。
#[derive(Debug)]
pub struct OperationReporter<P, I> {
    state: Arc<State<P, I>>,
}

#[derive(Debug)]
struct State<P, I> {
    queue: Mutex<Queue<P, I>>,
    readable: Condvar,
    writable: Condvar,
    cancelled: AtomicBool,
    closed: AtomicBool,
}

#[derive(Debug)]
struct Queue<P, I> {
    events: VecDeque<Event<P, I>>,
    pending_progress: Option<P>,
    terminal_enqueued: bool,
    terminal_consumed: bool,
}

impl<P, I> State<P, I> {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Queue {
                events: VecDeque::with_capacity(EVENT_QUEUE_CAPACITY),
                pending_progress: None,
                terminal_enqueued: false,
                terminal_consumed: false,
            }),
            readable: Condvar::new(),
            writable: Condvar::new(),
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn lock_queue(&self) -> MutexGuard<'_, Queue<P, I>> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cannot_write(&self, queue: &Queue<P, I>) -> bool {
        self.closed.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire)
            || queue.terminal_enqueued
    }

    fn wait_writable<'a>(
        &self,
        mut queue: MutexGuard<'a, Queue<P, I>>,
    ) -> Option<MutexGuard<'a, Queue<P, I>>> {
        while queue.events.len() >= EVENT_QUEUE_CAPACITY {
            if self.cannot_write(&queue) {
                return None;
            }
            queue = self
                .writable
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        (!self.cannot_write(&queue)).then_some(queue)
    }

    fn flush_progress<'a>(
        &self,
        queue: MutexGuard<'a, Queue<P, I>>,
    ) -> Option<MutexGuard<'a, Queue<P, I>>> {
        if queue.pending_progress.is_none() {
            return Some(queue);
        }
        let mut queue = self.wait_writable(queue)?;
        if let Some(progress) = queue.pending_progress.take() {
            queue.events.push_back(Event::Progress(progress));
            self.readable.notify_one();
        }
        Some(queue)
    }

    fn enqueue_terminal(&self, terminal: Event<P, I>) {
        let mut queue = self.lock_queue();
        if self.closed.load(Ordering::Acquire) || queue.terminal_enqueued {
            return;
        }
        if queue.pending_progress.is_some() {
            let Some(next) = self.wait_for_terminal_space(queue) else {
                return;
            };
            queue = next;
            if let Some(progress) = queue.pending_progress.take() {
                queue.events.push_back(Event::Progress(progress));
                self.readable.notify_one();
            }
        }
        let Some(mut queue) = self.wait_for_terminal_space(queue) else {
            return;
        };
        queue.events.push_back(terminal);
        queue.terminal_enqueued = true;
        self.readable.notify_all();
    }

    fn wait_for_terminal_space<'a>(
        &self,
        mut queue: MutexGuard<'a, Queue<P, I>>,
    ) -> Option<MutexGuard<'a, Queue<P, I>>> {
        while queue.events.len() >= EVENT_QUEUE_CAPACITY {
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            queue = self
                .writable
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        (!self.closed.load(Ordering::Acquire)).then_some(queue)
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut queue = self.lock_queue();
        queue.events.clear();
        queue.pending_progress = None;
        self.readable.notify_all();
        self.writable.notify_all();
    }
}

impl<P, I> OperationReporter<P, I> {
    /// 发布最新进度；尚未消费的连续进度会被此值替换。
    pub fn progress(&self, progress: P) -> bool {
        let mut queue = self.state.lock_queue();
        if self.state.cannot_write(&queue) {
            return false;
        }
        queue.pending_progress = Some(progress);
        self.state.readable.notify_one();
        true
    }

    /// 发布不可丢弃的结果；队列满时阻塞并施加背压。
    pub fn item(&self, item: I) -> bool {
        let queue = self.state.lock_queue();
        let Some(queue) = self.state.flush_progress(queue) else {
            return false;
        };
        let Some(mut queue) = self.state.wait_writable(queue) else {
            return false;
        };
        queue.events.push_back(Event::Item(item));
        self.state.readable.notify_one();
        true
    }

    /// 调用方是否已经请求取消。
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// 消费端是否已经关闭或丢弃句柄。
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::Acquire)
    }

    /// 在指定时间内等待取消或关闭，返回是否应停止生产。
    pub fn wait_for_stop(&self, timeout: Duration) -> bool {
        if self.is_cancelled() || self.is_closed() {
            return true;
        }
        let queue = self.state.lock_queue();
        let (_queue, _) = self
            .state
            .readable
            .wait_timeout(queue, timeout)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.is_cancelled() || self.is_closed()
    }
}

/// 启动一个使用统一协议的长任务。
#[must_use]
pub fn start_operation<P, I, F>(producer: F) -> OperationHandle<P, I>
where
    P: Send + 'static,
    I: Send + 'static,
    F: FnOnce(OperationReporter<P, I>) -> std::result::Result<(), String> + Send + 'static,
{
    let state = Arc::new(State::new());
    let worker_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let reporter = OperationReporter {
            state: Arc::clone(&worker_state),
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| producer(reporter)));
        let terminal = if worker_state.cancelled.load(Ordering::Acquire) {
            Event::Cancelled
        } else {
            match outcome {
                Ok(Ok(())) => Event::Done,
                Ok(Err(message)) => Event::Failed { message },
                Err(payload) => Event::Failed {
                    message: panic_message(payload),
                },
            }
        };
        worker_state.enqueue_terminal(terminal);
    });
    OperationHandle { state }
}

/// 等待下一事件；超时返回 `None` 且不消费任何事件。
pub fn next_event<P, I>(handle: &OperationHandle<P, I>, timeout_ms: u64) -> Option<Event<P, I>> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut queue = handle.state.lock_queue();
    loop {
        if let Some(event) = queue.events.pop_front() {
            if event.is_terminal() {
                queue.terminal_consumed = true;
            }
            handle.state.writable.notify_one();
            return Some(event);
        }
        if let Some(progress) = queue.pending_progress.take() {
            return Some(Event::Progress(progress));
        }
        if queue.terminal_consumed || handle.state.closed.load(Ordering::Acquire) {
            return None;
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let timeout = deadline.saturating_duration_since(now);
        let (next_queue, wait) = wait_timeout(&handle.state.readable, queue, timeout);
        queue = next_queue;
        if wait.timed_out() && queue.events.is_empty() && queue.pending_progress.is_none() {
            return None;
        }
    }
}

/// 幂等地请求取消任务。
pub fn cancel<P, I>(handle: &OperationHandle<P, I>) {
    handle.state.cancelled.store(true, Ordering::Release);
    handle.state.readable.notify_all();
    handle.state.writable.notify_all();
}

/// 幂等地关闭句柄并释放待消费事件。
pub fn close<P, I>(handle: &OperationHandle<P, I>) {
    handle.state.close();
}

impl<P, I> Drop for OperationHandle<P, I> {
    fn drop(&mut self) {
        self.state.close();
    }
}

fn wait_timeout<'a, T>(
    condition: &Condvar,
    guard: MutexGuard<'a, T>,
    timeout: Duration,
) -> (MutexGuard<'a, T>, WaitTimeoutResult) {
    condition
        .wait_timeout(guard, timeout)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return format!("长任务生产者异常退出：{message}");
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return format!("长任务生产者异常退出：{message}");
    }
    "长任务生产者异常退出".to_owned()
}

/// 可由传输适配器复用的协议一致性测试。
pub mod testing {
    use super::{
        Event, OperationHandle, OperationReporter, cancel, close, next_event, start_operation,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    /// 判定「事件流挂死」而不是「这台机器慢」的上限。
    ///
    /// **它不是延迟承诺。** 协议本身允许 `next_event` 在超时时返回 `None` 且不消费任何
    /// 事件（`assert_timeout_does_not_consume` 就在断言这件事），所以「一次等待没拿到
    /// 事件」只说明这一轮没等到，不说明流死了。判定挂起只能靠反复等待到一个足够宽的上限。
    ///
    /// 取值有上下两条边界，不是随手乘出来的：
    /// - **下界**：必须严格大于本模块里所有延迟承诺（当前最大是 `CANCEL_PROMPTNESS`）。
    ///   否则一台被抢占的机器会先在这里被误判成「挂起」，而不是在那条延迟断言上失败并
    ///   给出对应诊断。下面的 `const` 断言把这条关系钉死，调小它会编译失败。
    /// - **上界**：真挂死时必须在秒级失败而不是拖到工作流超时。八条子断言各自最坏等一轮，
    ///   总计仍在一分钟内，而正常一轮是毫秒级。
    const LIVENESS_BUDGET: Duration = Duration::from_secs(5);

    /// 取消到终态的上限。**这一条是真的延迟承诺**：脚本化生产者以 10 ms 粒度轮询停止信号，
    /// 因此这里留了 50 倍余量；放宽它等于放宽协议本身，不能拿它去兜调度噪声。
    const CANCEL_PROMPTNESS_MS: u64 = 500;
    const CANCEL_PROMPTNESS: Duration = Duration::from_millis(CANCEL_PROMPTNESS_MS);

    const _: () = assert!(
        LIVENESS_BUDGET.as_millis() > CANCEL_PROMPTNESS.as_millis(),
        "挂起判定必须比延迟承诺宽，否则慢机器会在挂起断言上失败、掩盖真正的延迟回归"
    );

    /// 等待挂起预算时的单轮长度。切片重试而不是一次长等：这样「超时不消费」的协议语义
    /// 在等待期间被反复走到，而不是只在一次调用里走一遍。
    const LIVENESS_POLL_MS: u64 = 200;

    /// 核心测试与下游传输适配器共同实现的最小接口。
    pub trait ConformanceAdapter {
        /// 被测句柄。
        type Handle;

        /// 启动脚本化生产者。
        fn start<F>(&self, producer: F) -> Self::Handle
        where
            F: FnOnce(OperationReporter<u16, u16>) -> std::result::Result<(), String>
                + Send
                + 'static;

        /// 等待下一事件。
        fn next_event(&self, handle: &Self::Handle, timeout_ms: u64) -> Option<Event<u16, u16>>;

        /// 请求取消。
        fn cancel(&self, handle: &Self::Handle);

        /// 关闭句柄。
        fn close(&self, handle: &Self::Handle);
    }

    /// 直接测试核心协议的适配器。
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CoreAdapter;

    impl ConformanceAdapter for CoreAdapter {
        type Handle = OperationHandle<u16, u16>;

        fn start<F>(&self, producer: F) -> Self::Handle
        where
            F: FnOnce(OperationReporter<u16, u16>) -> std::result::Result<(), String>
                + Send
                + 'static,
        {
            start_operation(producer)
        }

        fn next_event(&self, handle: &Self::Handle, timeout_ms: u64) -> Option<Event<u16, u16>> {
            next_event(handle, timeout_ms)
        }

        fn cancel(&self, handle: &Self::Handle) {
            cancel(handle);
        }

        fn close(&self, handle: &Self::Handle) {
            close(handle);
        }
    }

    /// 对适配器执行完整的统一协议断言。
    pub fn assert_conforms<A>(adapter: &A)
    where
        A: ConformanceAdapter,
    {
        assert_ordering_and_single_terminal(adapter);
        assert_progress_coalescing(adapter);
        assert_bounded_backpressure(adapter);
        assert_timeout_does_not_consume(adapter);
        assert_cancel_is_idempotent_and_prompt(adapter);
        assert_close_after_terminal_is_safe(adapter);
        assert_drop_closes_producer(adapter);
        assert_producer_death_fails(adapter);
    }

    fn assert_ordering_and_single_terminal<A: ConformanceAdapter>(adapter: &A) {
        let handle = adapter.start(|reporter| {
            assert!(reporter.progress(1));
            assert!(reporter.item(10));
            assert!(reporter.progress(2));
            assert!(reporter.item(20));
            Ok(())
        });
        let events = collect_through_terminal(adapter, &handle);
        assert_eq!(
            events,
            [
                Event::Progress(1),
                Event::Item(10),
                Event::Progress(2),
                Event::Item(20),
                Event::Done,
            ]
        );
        assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
        assert_eq!(adapter.next_event(&handle, 1), None);
    }

    fn assert_progress_coalescing<A: ConformanceAdapter>(adapter: &A) {
        let produced = Arc::new(AtomicBool::new(false));
        let producer_probe = Arc::clone(&produced);
        let handle = adapter.start(move |reporter| {
            for value in 0..1_000 {
                assert!(reporter.progress(value));
            }
            producer_probe.store(true, Ordering::Release);
            Ok(())
        });
        wait_until(|| produced.load(Ordering::Acquire));
        let events = collect_through_terminal(adapter, &handle);
        assert_eq!(events, [Event::Progress(999), Event::Done]);
    }

    fn assert_bounded_backpressure<A: ConformanceAdapter>(adapter: &A) {
        let produced = Arc::new(AtomicUsize::new(0));
        let producer_probe = Arc::clone(&produced);
        let handle = adapter.start(move |reporter| {
            for value in 0..=256 {
                if !reporter.item(value) {
                    return Ok(());
                }
                producer_probe.fetch_add(1, Ordering::Release);
            }
            Ok(())
        });
        wait_until(|| produced.load(Ordering::Acquire) >= 256);
        assert_eq!(produced.load(Ordering::Acquire), 256);
        assert_eq!(
            await_event(adapter, &handle, "队列已满，第一条必须是最早的 Item"),
            Event::Item(0)
        );
        wait_until(|| produced.load(Ordering::Acquire) == 257);
        adapter.close(&handle);
    }

    fn assert_timeout_does_not_consume<A: ConformanceAdapter>(adapter: &A) {
        // 用闸门而不是 `sleep`：靠「生产者睡 40 ms」来保证首次 1 ms 等待落在事件之前，
        // 是在和调度器赛跑——测试线程只要在 start 之后被抢占 40 ms，事件就已经在队里了，
        // 那次 1 ms 等待会拿到 Item(7) 而不是 None。闸门让「此刻还没有事件」成为事实，
        // 于是「超时不消费」由后面那次等待仍能拿到完整的 Item(7) 来证明。
        let gate = Arc::new(Gate::default());
        let producer_gate = Arc::clone(&gate);
        let handle = adapter.start(move |reporter| {
            producer_gate.wait_for_release();
            assert!(reporter.item(7));
            Ok(())
        });
        assert_eq!(adapter.next_event(&handle, 1), None);
        gate.release();
        assert_eq!(
            await_event(adapter, &handle, "放行后必须拿到未被超时吞掉的事件"),
            Event::Item(7)
        );
        assert_eq!(
            await_event(adapter, &handle, "生产者返回后必须给出终态"),
            Event::Done
        );
    }

    fn assert_cancel_is_idempotent_and_prompt<A: ConformanceAdapter>(adapter: &A) {
        let handle = adapter.start(|reporter| {
            while !reporter.wait_for_stop(Duration::from_millis(10)) {}
            Ok(())
        });
        let started = Instant::now();
        adapter.cancel(&handle);
        adapter.cancel(&handle);
        // 这里刻意仍是延迟断言，不走挂起预算：脚本化生产者自己永远不会结束，所以拿到
        // Cancelled 就已经证明取消被响应，而 `CANCEL_PROMPTNESS` 进一步要求它是及时的。
        assert_eq!(
            adapter.next_event(&handle, CANCEL_PROMPTNESS_MS),
            Some(Event::Cancelled)
        );
        assert!(started.elapsed() <= CANCEL_PROMPTNESS);
        assert_eq!(adapter.next_event(&handle, 1), None);
    }

    fn assert_close_after_terminal_is_safe<A: ConformanceAdapter>(adapter: &A) {
        let handle = adapter.start(|_| Ok(()));
        assert_eq!(
            await_event(adapter, &handle, "无事可做的生产者必须直接给出终态"),
            Event::Done
        );
        adapter.close(&handle);
        adapter.close(&handle);
        assert_eq!(adapter.next_event(&handle, 1), None);
    }

    fn assert_drop_closes_producer<A: ConformanceAdapter>(adapter: &A) {
        let stopped = Arc::new(AtomicBool::new(false));
        let producer_probe = Arc::clone(&stopped);
        let handle = adapter.start(move |reporter| {
            let mut value = 0;
            while reporter.item(value) {
                value = value.wrapping_add(1);
            }
            producer_probe.store(true, Ordering::Release);
            Ok(())
        });
        drop(handle);
        wait_until(|| stopped.load(Ordering::Acquire));
    }

    fn assert_producer_death_fails<A: ConformanceAdapter>(adapter: &A) {
        let handle = adapter
            .start(|_| -> std::result::Result<(), String> { panic!("scripted producer death") });
        let event = await_event(adapter, &handle, "生产者死亡必须生成终态而不是挂起");
        match event {
            Event::Failed { message } => assert!(message.contains("scripted producer death")),
            other => panic!("生产者死亡应得到 Failed，实际 {other:?}"),
        }
        assert_eq!(adapter.next_event(&handle, 1), None);
    }

    fn collect_through_terminal<A: ConformanceAdapter>(
        adapter: &A,
        handle: &A::Handle,
    ) -> Vec<Event<u16, u16>> {
        let mut events = Vec::new();
        loop {
            let event = await_event(adapter, handle, "事件流必须前进到终态");
            let terminal = event.is_terminal();
            events.push(event);
            if terminal {
                return events;
            }
        }
    }

    /// 反复等待直到拿到事件；只有整段 `LIVENESS_BUDGET` 都没等到才判为挂起。
    fn await_event<A: ConformanceAdapter>(
        adapter: &A,
        handle: &A::Handle,
        expectation: &str,
    ) -> Event<u16, u16> {
        let started = Instant::now();
        loop {
            if let Some(event) = adapter.next_event(handle, LIVENESS_POLL_MS) {
                return event;
            }
            assert!(
                started.elapsed() < LIVENESS_BUDGET,
                "{expectation}：{LIVENESS_BUDGET:?} 内一个事件都没有，判为挂起"
            );
        }
    }

    /// 等待生产者到达某个状态。**这是同步栅栏，不是延迟断言**：等多久不影响任何被断言的
    /// 性质（栅栏之后那句 `assert_eq!` 才是断言），所以统一用挂起预算，而不是各写一个
    /// 几百毫秒的数——那些数在被抢占的 CI runner 上只会变成随机失败。
    fn wait_until(condition: impl Fn() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < LIVENESS_BUDGET,
                "等待协议状态变化超过 {LIVENESS_BUDGET:?}，判为挂起"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 让脚本化生产者停在指定位置，直到测试放行。
    #[derive(Debug, Default)]
    struct Gate {
        released: Mutex<bool>,
        changed: Condvar,
    }

    impl Gate {
        fn wait_for_release(&self) {
            let started = Instant::now();
            let mut released = self.released.lock().expect("闸门可加锁");
            while !*released {
                assert!(
                    started.elapsed() < LIVENESS_BUDGET,
                    "闸门在 {LIVENESS_BUDGET:?} 内没有放行"
                );
                let (next, _) = self
                    .changed
                    .wait_timeout(released, Duration::from_millis(LIVENESS_POLL_MS))
                    .expect("闸门可等待");
                released = next;
            }
        }

        fn release(&self) {
            *self.released.lock().expect("闸门可加锁") = true;
            self.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{CoreAdapter, assert_conforms};

    #[test]
    fn core_operation_protocol_conforms() {
        assert_conforms(&CoreAdapter);
    }
}
