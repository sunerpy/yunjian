#![cfg(feature = "uniffi")]

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use yunjian_core::operation::testing::{ConformanceAdapter, assert_conforms};
use yunjian_core::operation::{Event, OperationReporter, start_operation};
use yunjian_mobile::uniffi_native::{NativeEventSink, NativeOperation};

#[derive(Debug, Clone, Copy)]
struct UniffiAdapter;

impl ConformanceAdapter for UniffiAdapter {
    type Handle = Arc<NativeOperation>;

    fn start<F>(&self, producer: F) -> Self::Handle
    where
        F: FnOnce(OperationReporter<u16, u16>) -> Result<(), String> + Send + 'static,
    {
        NativeOperation::from_operation(start_operation(producer))
    }

    fn next_event(&self, handle: &Self::Handle, timeout_ms: u64) -> Option<Event<u16, u16>> {
        handle
            .next_event(timeout_ms)
            .map(|json| serde_json::from_str(&json).expect("UniFFI 事件必须保持核心 JSON 契约"))
    }

    fn cancel(&self, handle: &Self::Handle) {
        handle.cancel();
    }

    fn close(&self, handle: &Self::Handle) {
        handle.close();
    }
}

#[test]
fn uniffi_pull_adapter_obeys_all_core_protocol_rules() {
    assert_conforms(&UniffiAdapter);
}

struct RecordingSink {
    events: Arc<(Mutex<Vec<String>>, Condvar)>,
}

impl NativeEventSink for RecordingSink {
    fn on_event(&self, event_json: String) {
        self.events
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event_json);
        self.events.1.notify_all();
    }
}

#[test]
fn callback_variant_delivers_the_same_order_through_one_terminal_event() {
    let operation = NativeOperation::from_operation(start_operation(|reporter| {
        assert!(reporter.progress(1_u16));
        assert!(reporter.item(10_u16));
        assert!(reporter.progress(2_u16));
        assert!(reporter.item(20_u16));
        Ok(())
    }));
    let events = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    operation.clone().subscribe(Box::new(RecordingSink {
        events: Arc::clone(&events),
    }));

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut delivered = events
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while delivered.len() < 5 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "回调事件流未在五秒内到达终态");
        delivered = events
            .1
            .wait_timeout(delivered, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
    let events = delivered
        .iter()
        .map(|json| serde_json::from_str::<Event<u16, u16>>(json).expect("回调事件应可解析"))
        .collect::<Vec<_>>();
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
}

#[test]
fn callback_variant_waits_across_empty_poll_windows() {
    let operation = NativeOperation::from_operation(start_operation(
        |reporter: OperationReporter<u16, u16>| {
            std::thread::sleep(Duration::from_millis(450));
            assert!(reporter.item(42_u16));
            Ok(())
        },
    ));
    let events = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    operation.clone().subscribe(Box::new(RecordingSink {
        events: Arc::clone(&events),
    }));

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut delivered = events
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while delivered.len() < 2 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "慢生产者的回调事件未在五秒内到达终态");
        delivered = events
            .1
            .wait_timeout(delivered, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
    let events = delivered
        .iter()
        .map(|json| serde_json::from_str::<Event<u16, u16>>(json).expect("回调事件应可解析"))
        .collect::<Vec<_>>();
    assert_eq!(events, [Event::Item(42), Event::Done]);
}

#[test]
fn uniffi_event_bytes_are_identical_to_the_tauri_channel_contract() {
    let operation = NativeOperation::from_operation(start_operation(|reporter| {
        assert!(reporter.progress(7_u16));
        assert!(reporter.item(11_u16));
        Ok(())
    }));

    for tauri_event in [Event::Progress(7), Event::Item(11), Event::Done] {
        let uniffi_json = operation.next_event(5_000).expect("脚本事件应到达");
        let tauri_json =
            serde_json::to_string(&tauri_event).expect("Tauri Channel 使用 serde JSON");
        assert_eq!(uniffi_json.as_bytes(), tauri_json.as_bytes());
    }
}
