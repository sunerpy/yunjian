use crate::Appreciation;

/// 接收且只接收完整赏析的缓存写入边界。
pub trait AppreciationCacheWriter: Send + Sync {
    /// 原子写入一个已经完整结束的生成结果。
    fn store_completed(&self, key: &str, appreciation: &Appreciation) -> yunjian_core::Result<()>;
}

#[cfg(test)]
mod tests {
    use crate::genai_provider::{GenAiProvider, GenAiProviderConfig, ProviderKind};
    use crate::provider::tests::fixture_detail;
    use crate::{
        Appreciation, AppreciationCacheWriter, AppreciationProvider, AppreciationRequest,
        AppreciationStreamItem,
    };
    use secrecy::SecretString;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};
    use yunjian_core::operation::testing::{ConformanceAdapter, assert_conforms};
    use yunjian_core::operation::{
        Event, OperationHandle, OperationReporter, cancel, close, next_event, start_operation,
    };

    const PROBE_KEY: &str = "sk-STREAMTEST123";

    #[derive(Debug, Clone, Default)]
    struct MemoryCache {
        entries: Arc<Mutex<BTreeMap<String, Appreciation>>>,
    }

    impl MemoryCache {
        fn len(&self) -> usize {
            self.entries.lock().expect("cache lock").len()
        }

        fn get(&self, key: &str) -> Option<Appreciation> {
            self.entries.lock().expect("cache lock").get(key).cloned()
        }
    }

    impl AppreciationCacheWriter for MemoryCache {
        fn store_completed(
            &self,
            key: &str,
            appreciation: &Appreciation,
        ) -> yunjian_core::Result<()> {
            self.entries
                .lock()
                .expect("cache lock")
                .insert(key.to_owned(), appreciation.clone());
            Ok(())
        }
    }

    fn provider(base_url: &str, cache: MemoryCache) -> GenAiProvider {
        GenAiProvider::with_secret(
            GenAiProviderConfig::new(ProviderKind::OpenAI).with_base_url(base_url.to_owned()),
            Some(SecretString::from(PROBE_KEY.to_owned())),
        )
        .expect("构造流式 provider")
        .with_cache_writer(Arc::new(cache))
    }

    fn request() -> AppreciationRequest {
        AppreciationRequest::new(fixture_detail(), "gpt-4o-mini")
    }

    fn read_request(stream: &TcpStream) {
        let mut reader = BufReader::new(stream);
        let mut content_length = 0_usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("读取请求头");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().expect("Content-Length 数字");
            }
        }
        let mut body = vec![0_u8; content_length];
        reader.read_exact(&mut body).expect("读取请求体");
    }

    fn write_headers(stream: &mut TcpStream, content_length: usize) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
        )
        .expect("写响应头");
        stream.flush().expect("刷新响应头");
    }

    fn content_chunk(text: &str, finish_reason: Option<&str>) -> String {
        let finish_reason = finish_reason
            .map(|reason| format!(r#""{reason}""#))
            .unwrap_or_else(|| "null".to_owned());
        format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{text}\"}},\"finish_reason\":{finish_reason}}}]}}\n\n"
        )
    }

    fn usage_tail() -> &'static str {
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}\n\ndata: [DONE]\n\n"
    }

    fn bind_server() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定流探针端口");
        let address = listener.local_addr().expect("读取流探针地址");
        (listener, format!("http://{address}"))
    }

    fn next_item(
        handle: &OperationHandle<crate::AppreciationProgress, AppreciationStreamItem>,
        timeout_ms: u64,
    ) -> Event<crate::AppreciationProgress, AppreciationStreamItem> {
        next_event(handle, timeout_ms).expect("事件流应在时限内前进")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_chunk_arrives_before_server_finishes_then_usage_and_complete_are_cached() {
        let (listener, base_url) = bind_server();
        let (release_tx, release_rx) = mpsc::channel();
        let server_finished = Arc::new(AtomicBool::new(false));
        let finished_probe = Arc::clone(&server_finished);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("接受流连接");
            read_request(&stream);
            let first = content_chunk("大江", None);
            let rest = format!("{}{}", content_chunk("东去", Some("stop")), usage_tail());
            write_headers(&mut stream, first.len() + rest.len());
            stream.write_all(first.as_bytes()).expect("写首片");
            stream.flush().expect("刷新首片");
            release_rx.recv().expect("等待测试放行尾片");
            stream.write_all(rest.as_bytes()).expect("写尾片");
            stream.flush().expect("刷新尾片");
            finished_probe.store(true, Ordering::Release);
        });

        let cache = MemoryCache::default();
        let provider = provider(&base_url, cache.clone());
        let request = request();
        let cache_key = request.cache_key(&provider.id());
        let handle = provider
            .appreciate_stream(request)
            .await
            .expect("启动流式赏析");

        assert!(matches!(
            next_item(&handle, 500),
            Event::Item(AppreciationStreamItem::Chunk(text)) if text == "大江"
        ));
        assert!(
            !server_finished.load(Ordering::Acquire),
            "首片必须在服务端完成发送前抵达，不能先缓冲完整响应"
        );
        release_tx.send(()).expect("放行尾片");

        let mut completed = None;
        loop {
            match next_item(&handle, 500) {
                Event::Item(AppreciationStreamItem::Complete(appreciation)) => {
                    completed = Some(appreciation)
                }
                Event::Done => break,
                Event::Progress(_) | Event::Item(AppreciationStreamItem::Chunk(_)) => {}
                other => panic!("正常流得到意外事件：{other:?}"),
            }
        }
        server.join().expect("回收正常流服务端");

        let completed = completed.expect("正常流必须发出 Complete");
        assert_eq!(completed.text, "大江东去");
        let usage = completed.usage.expect("服务端用量必须透出");
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.total_tokens),
            (11, 7, 18)
        );
        assert_eq!(
            cache.get(&cache_key).expect("正常完成必须写缓存"),
            completed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_mid_stream_stops_chunks_within_100_ms_and_never_caches_partial_text() {
        let (listener, base_url) = bind_server();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("接受取消流连接");
            read_request(&stream);
            let first = content_chunk("首片", None);
            let later = (0..20)
                .map(|index| content_chunk(&format!("尾片{index}"), None))
                .collect::<String>();
            let end = format!("{}{}", content_chunk("终片", Some("stop")), usage_tail());
            write_headers(&mut stream, first.len() + later.len() + end.len());
            stream.write_all(first.as_bytes()).expect("写取消流首片");
            stream.flush().expect("刷新取消流首片");
            for chunk in later.as_bytes().chunks(64) {
                thread::sleep(Duration::from_millis(20));
                if stream.write_all(chunk).is_err() || stream.flush().is_err() {
                    return;
                }
            }
            let _ = stream.write_all(end.as_bytes());
            let _ = stream.flush();
        });

        let cache = MemoryCache::default();
        let handle = provider(&base_url, cache.clone())
            .appreciate_stream(request())
            .await
            .expect("启动可取消流");
        assert!(matches!(
            next_item(&handle, 500),
            Event::Item(AppreciationStreamItem::Chunk(text)) if text == "首片"
        ));

        let cancelled_at = Instant::now();
        cancel(&handle);
        let mut extra_chunks = 0_usize;
        loop {
            match next_item(&handle, 100) {
                Event::Cancelled => break,
                Event::Item(AppreciationStreamItem::Chunk(_)) => extra_chunks += 1,
                Event::Progress(_) => {}
                other => panic!("取消流得到意外事件：{other:?}"),
            }
        }
        assert!(
            cancelled_at.elapsed() <= Duration::from_millis(100),
            "取消后 100 ms 内必须终止转发并丢弃 HTTP 流"
        );
        assert_eq!(extra_chunks, 0, "取消后不得再向 sink 写文本片段");
        assert_eq!(cache.len(), 0, "取消的部分结果不得进入缓存");
        server.join().expect("回收取消流服务端");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn malformed_mid_stream_transport_is_typed_failed_and_never_cached() {
        let (listener, base_url) = bind_server();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("接受错误流连接");
            read_request(&stream);
            let first = content_chunk("有效首片", None);
            let malformed = "data: {not-json}\n\n";
            write_headers(&mut stream, first.len() + malformed.len());
            stream.write_all(first.as_bytes()).expect("写错误流首片");
            stream.write_all(malformed.as_bytes()).expect("写畸形尾片");
            stream.flush().expect("刷新错误流");
        });

        let cache = MemoryCache::default();
        let handle = provider(&base_url, cache.clone())
            .appreciate_stream(request())
            .await
            .expect("启动错误流");
        assert!(matches!(
            next_item(&handle, 500),
            Event::Item(AppreciationStreamItem::Chunk(text)) if text == "有效首片"
        ));
        loop {
            match next_item(&handle, 500) {
                Event::Failed { message } => {
                    assert!(
                        message.contains("AI 错误"),
                        "失败必须保留类型化 AI 边界：{message}"
                    );
                    break;
                }
                Event::Progress(_) => {}
                other => panic!("错误流得到意外事件：{other:?}"),
            }
        }
        assert_eq!(cache.len(), 0, "传输错误不得缓存部分结果");
        server.join().expect("回收错误流服务端");
    }

    #[derive(Debug, Clone, Copy)]
    struct AppreciationOperationAdapter;

    impl ConformanceAdapter for AppreciationOperationAdapter {
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

    #[test]
    fn appreciation_operation_adapter_obeys_all_core_protocol_rules() {
        assert_conforms(&AppreciationOperationAdapter);
    }
}
