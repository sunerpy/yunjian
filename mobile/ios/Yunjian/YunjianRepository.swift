import Foundation
import YunjianMobile

/// 应用私有存储中的固定布局。
///
/// 与 Android 的 `YunjianPaths` 同一口径：全部落在应用容器内的 Application Support 之下。
/// iOS 没有「外部存储」这一层，所以 Android 那条「模型优先放外部私有目录」的分支在这里
/// 收敛成同一个根——**这不是简化，是平台上没有第二个位置**。
struct YunjianPaths {
    let appDataDir: URL
    let corpusDataDir: URL
    let schedulerPath: URL

    init() {
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first ?? URL(fileURLWithPath: NSTemporaryDirectory())
        appDataDir = base.appendingPathComponent("yunjian", isDirectory: true)
        corpusDataDir = appDataDir.appendingPathComponent("corpus", isDirectory: true)
        schedulerPath = appDataDir.appendingPathComponent("recite.db")
    }

    func ensure() {
        try? FileManager.default.createDirectory(at: corpusDataDir, withIntermediateDirectories: true)
    }

    /// 语料是否已落在生产位置。首启判断只看这一件事（与 Android 同）。
    var corpusPresent: Bool {
        FileManager.default.fileExists(atPath: corpusDataDir.appendingPathComponent("corpus.db").path)
    }

    /// 语音权重的落点。目录名与 `yunjian-voice::models` 的缓存布局一致。
    var modelRoot: URL { appDataDir.appendingPathComponent("models", isDirectory: true) }
}

/// 首启物化的一条进度。字段与内核事件同名，不在这里另起口径。
struct MaterializationStage: Equatable {
    let stage: String
    let detail: String
    let fraction: Double?
}

/// 一条搜索结果。
struct SearchHit: Identifiable, Equatable {
    let poemId: String
    let title: String
    let author: String
    let snippet: String

    var id: String { poemId }
}

/// 集评一条，带出处。
struct Commentary: Equatable {
    let text: String
    let sourceTitle: String
    let sourceLocator: String
}

/// 随包 AI 赏析。`reviewed` 由 Rust 侧钉在 `false`，此处只读。
struct ShippedAppreciation: Equatable {
    let text: String
    let model: String
    let source: String
    let reviewed: Bool
}

/// 阅读页要显示的一切。
struct PoemReading: Equatable {
    let poemId: String
    let title: String
    let author: String
    let dynasty: String
    let body: String
    let commentaries: [Commentary]
    let appreciation: ShippedAppreciation?
}

/// 一次打字背诵的题目。
struct ReciteSession: Equatable {
    let poemId: String
    let prompt: String
    let lineCount: Int
}

/// 一次打字背诵的评分。
struct ReciteScore: Equatable {
    let completeness: Double
    let accuracyStrict: Double
    let accuracyLenient: Double
    let isRejected: Bool
    let normalCount: Int
    let deletionCount: Int
    let insertionCount: Int
    let substitutionCount: Int
}

/// 唯一允许触达 Rust 的地方。
///
/// # 为什么把 JSON 解析集中在这里
///
/// 与 Android 的 `YunjianRepository` 同一理由：UniFFI 边界收发 `String` JSON（领域 serde
/// 契约原样穿过）。把 `JSONSerialization` 散在各个 View 里，等于让每个界面各自猜一遍字段名；
/// 集中之后字段名只有一处会与 Rust 漂移，且漂移的表现是这里抛错而不是某个界面静默空白。
///
/// # 字段名全部取自 Rust 的 serde 契约，与 Android 侧逐字相同
///
/// `hits`（不是 `items`）、`snippet.text`、`citation.dynasty.canonical`、
/// `citation.work_completed_by`、`score.accuracy_strict` —— 这些名字在两个平台上必须一致，
/// 否则同一条判据在一个平台上量到值、在另一个平台上永远量成空。
///
/// # 尚未由 Xcode 编译验证
///
/// 本机是 Linux，没有 Xcode，因此本文件与同目录其余 Swift 只经过人工与结构断言校验，
/// **没有经过 Swift 编译器**。见 `mobile/ios/README.md` 的「未验证清单」。
final class YunjianRepository: @unchecked Sendable {
    /// 进程内唯一那份 repository。
    ///
    /// 与 Android 同一理由：每份 repository 会惰性构造自己的 `NativeFacade`，而门面同时打开
    /// 语料与复习库两个 SQLite 文件。两份门面持有同一个文件时写入方报 `database is locked`
    /// —— Android 真机实测这条错误曾把十条断言里的六条连带拖红，而它的文字完全不提「有两份」。
    static let shared = YunjianRepository()

    let paths = YunjianPaths()

    private let lock = NSLock()
    private var facade: NativeFacade?
    private let materialization = Materialization()

    private init() {}

    var corpusPresent: Bool { paths.corpusPresent }

    /// 门面配置。`provider = none` 是「不配置任何生成供应商」——随包赏析走
    /// `shippedAppreciation`，那条路径不需要 key，所以首启就能显示赏析。
    private func configJson() -> String {
        let payload: [String: Any] = [
            "corpus_data_dir": paths.corpusDataDir.path,
            "scheduler_path": paths.schedulerPath.path,
            "app_data_dir": paths.appDataDir.path,
            "provider": "none",
        ]
        let data = (try? JSONSerialization.data(withJSONObject: payload)) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    /// 走生产路径下载、校验并原子物化语料与随包赏析种子。
    ///
    /// **进程内至多一次真的在跑**（见 `Materialization`）。重复调用只登记为订阅者并收到
    /// 已知状态的回放——两次并发物化会在 corpus.db 上撞成 `database is locked`。
    /// Android 侧把守卫从 ViewModel 提到进程级正是因为这个竞态；SwiftUI 的 `@StateObject`
    /// 在场景重建时同样会新建 ViewModel，所以这里必须做同一件事。
    func materialize(
        onStage: @escaping (MaterializationStage) -> Void,
        onDone: @escaping (String?) -> Void
    ) {
        guard materialization.claim(onStage: onStage, onDone: onDone) else { return }
        paths.ensure()
        do {
            let operation = try materializeAssets(configJson: configJson())
            let sink = MaterializationSink(
                onStage: { [weak self] stage in self?.materialization.publish(stage: stage) },
                onDone: { [weak self] failure in self?.materialization.publish(done: failure) },
                readStage: { [weak self] payload in self?.readStage(payload) },
                readSummary: { [weak self] payload in self?.readSummary(payload) }
            )
            // 句柄必须被持有：subscribe 之后事件从 Rust 工作线程回调，操作对象一旦释放
            // 事件流随之结束，界面会停在最后一条进度上——那比空白更糟（陈旧的话）。
            sink.retain(operation: operation)
            operation.subscribe(sink: sink)
        } catch {
            materialization.publish(done: Self.describe(error))
        }
    }

    /// 惰性构造门面。语料不在时构造必然失败，所以由调用方先完成物化。
    func open() throws -> NativeFacade {
        lock.lock()
        defer { lock.unlock() }
        if let facade { return facade }
        let created = try NativeFacade(configJson: configJson())
        facade = created
        return created
    }

    func corpusStatusJson() throws -> String { try open().corpusStatus() }

    /// 正文检索。命中数组是 `hits`，命中句在 `snippet.text`。
    func searchText(query: String, limit: Int = 20) throws -> [SearchHit] {
        let request = Self.json(["query": query, "limit": limit])
        let page = try Self.object(try open().searchText(requestJson: request))
        let hits = page["hits"] as? [[String: Any]] ?? []
        return hits.map { hit in
            SearchHit(
                poemId: hit["poem_id"] as? String ?? "",
                title: hit["title"] as? String ?? "",
                author: hit["author"] as? String ?? "",
                snippet: (hit["snippet"] as? [String: Any])?["text"] as? String ?? ""
            )
        }
    }

    /// 阅读页数据。
    ///
    /// 集评的出处在 `citation` 对象里，而且 `citation` **不是** `Option`：缺出处的集评根本
    /// 不会构造出来（内核直接返回 `CommentaryCitationMissing`）。因此这里不需要
    /// 「没有出处怎么显示」这条分支——与 Android 侧同一判断。
    func reading(poemId: String) throws -> PoemReading {
        let request = Self.json(["poem_id": poemId])
        let detail = try Self.object(try open().poemDetail(requestJson: request))
        let poem = detail["poem"] as? [String: Any] ?? [:]
        let commentaries = (detail["commentaries"] as? [[String: Any]] ?? []).map { entry -> Commentary in
            let citation = entry["citation"] as? [String: Any] ?? [:]
            let dynasty = (citation["dynasty"] as? [String: Any])?["canonical"] as? String ?? ""
            let author = citation["author"] as? String ?? ""
            let work = citation["work"] as? String ?? ""
            let note = citation["source_note"] as? String ?? ""
            let completedBy = citation["work_completed_by"] as? Int ?? 0
            return Commentary(
                text: entry["text"] as? String ?? "",
                sourceTitle: "\(dynasty)·\(author)《\(work)》",
                sourceLocator: "\(note)（成书不晚于 \(completedBy)）"
            )
        }
        return PoemReading(
            poemId: poemId,
            title: poem["title"] as? String ?? "",
            author: poem["author"] as? String ?? "",
            dynasty: (poem["dynasty"] as? [String: Any])?["canonical"] as? String ?? "",
            body: poem["body"] as? String ?? "",
            commentaries: commentaries,
            appreciation: try shippedAppreciation(poemId: poemId)
        )
    }

    /// 随包赏析。命中不需要 API key；未命中返回 `nil` 而不是抛错。
    func shippedAppreciation(poemId: String) throws -> ShippedAppreciation? {
        guard let raw = try open().shippedAppreciation(poemId: poemId, model: "shipped") else {
            return nil
        }
        let value = try Self.object(raw)
        return ShippedAppreciation(
            text: value["text"] as? String ?? "",
            model: value["model"] as? String ?? "",
            source: value["source"] as? String ?? "",
            reviewed: value["reviewed"] as? Bool ?? false
        )
    }

    func reciteStart(poemId: String) throws -> ReciteSession {
        let request = Self.json(["poem_id": poemId, "mode": ["mode": "first_char"]])
        let session = try Self.object(try open().reciteStart(requestJson: request))
        return ReciteSession(
            poemId: poemId,
            prompt: session["prompt"] as? String ?? "",
            lineCount: session["line_count"] as? Int ?? 0
        )
    }

    func reciteSubmit(poemId: String, answer: String, grade: String) throws -> ReciteScore {
        let request = Self.json(["poem_id": poemId, "answer": answer, "grade": grade])
        let submitted = try Self.object(try open().reciteSubmit(requestJson: request))
        let score = submitted["score"] as? [String: Any] ?? [:]
        return ReciteScore(
            completeness: score["completeness"] as? Double ?? 0,
            accuracyStrict: score["accuracy_strict"] as? Double ?? 0,
            accuracyLenient: score["accuracy_lenient"] as? Double ?? 0,
            isRejected: score["is_rejected"] as? Bool ?? false,
            normalCount: score["normal_count"] as? Int ?? 0,
            deletionCount: score["deletion_count"] as? Int ?? 0,
            insertionCount: score["insertion_count"] as? Int ?? 0,
            substitutionCount: score["substitution_count"] as? Int ?? 0
        )
    }

    /// 按需下载并校验一个语音模型，返回它在设备上的目录。
    ///
    /// 走产品自己那条路径（`ModelCache::ensure`：下载 + SHA-256 + 原子解包），由 Rust 写文件，
    /// 属主就是应用自己。安装包不含任何权重——这是产品的真实形态，两个平台一致。
    func fetchVoiceModel(modelName: String, onStage: @escaping (String) -> Void) -> String? {
        let root = paths.modelRoot
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        guard let operation = try? fetchVoiceModel(cacheRoot: root.path, modelName: modelName) else {
            onStage("模型下载无法启动")
            return nil
        }
        var directory: String?
        while true {
            // 一次轮询超时（`nil`）**不是终态**。把它当终态会让模型在解包中途被误判为完成，
            // 随后 `startAsr` 报「找不到 encoder」——那条报错不指向真因。
            guard let raw = operation.nextEvent(timeoutMs: 500) else { continue }
            guard let event = try? Self.object(raw) else { continue }
            let payload = event["payload"] as? [String: Any]
            switch event["type"] as? String {
            case "progress":
                onStage(Self.describeModelStage(payload))
            case "item":
                directory = payload?["directory"] as? String
            case "done":
                return directory
            case "failed":
                onStage("模型下载失败：\(payload?["message"] as? String ?? "")")
                return nil
            case "cancelled":
                return nil
            default:
                continue
            }
        }
    }

    /// 启动流式识别。
    ///
    /// `int8 = true` 是移动端的取值，与 Android 侧相同：fp32 那套权重 531 MiB，int8 是
    /// 189 MiB。`TransducerFiles::discover` 按这个开关挑 `.int8.onnx` 还是 `.onnx`，
    /// **挑错会报「找不到 encoder」而不是「精度不对」**。
    func startAsr(modelDir: String, reference: String, sampleRate: UInt32) throws -> NativeAsrOperation {
        try open().startAsr(modelDir: modelDir, int8: true, reference: reference, sampleRate: sampleRate)
    }

    // MARK: - 进度文案

    private func readStage(_ payload: [String: Any]) -> MaterializationStage {
        let stage = payload["stage"] as? String ?? ""
        switch stage {
        case "already_present":
            return MaterializationStage(stage: stage, detail: "语料已在本地", fraction: 1)
        case "verifying_archive":
            return MaterializationStage(
                stage: stage,
                detail: "正在核对归档摘要 · \(Self.mib(payload["bytes"]))",
                fraction: nil
            )
        case "archive_verified":
            return MaterializationStage(stage: stage, detail: "归档摘要一致", fraction: nil)
        case "decompressing":
            let done = Self.int(payload["bytes_done"])
            let total = Self.int(payload["bytes_total"])
            // `total == 0` 表示清单没给解压后大小，真机上就是这种情况。分母写 0 比不写更糟：
            // 它看起来像一个已知的总量，而那个总量是假的（Android 侧实测过这条文案）。
            return MaterializationStage(
                stage: stage,
                detail: total > 0
                    ? "正在解压语料库 · \(Self.mib(done)) / \(Self.mib(total))"
                    : "正在解压语料库 · 已写出 \(Self.mib(done))",
                fraction: total > 0 ? Double(done) / Double(total) : nil
            )
        case "materialized":
            return MaterializationStage(
                stage: stage,
                detail: "语料已原子落地 · \(payload["corpus_version"] as? String ?? "")",
                fraction: nil
            )
        case "deriving":
            let done = Self.int(payload["done"])
            let total = Self.int(payload["total"])
            let step = payload["step"] as? String ?? ""
            return MaterializationStage(
                stage: stage,
                detail: total > 0 ? "\(step)（\(done) / \(total) 首）" : step,
                fraction: total > 0 ? Double(done) / Double(total) : nil
            )
        case "derive_failed":
            return MaterializationStage(
                stage: stage,
                detail: "首启派生未完成：两字查询会退化，下次启动重试",
                fraction: nil
            )
        case "ready":
            let derived = payload["derived"] as? Bool ?? false
            return MaterializationStage(
                stage: stage,
                detail: "语料就绪 · \(payload["corpus_version"] as? String ?? "")"
                    + (derived ? " · 派生索引就绪" : " · 派生索引缺失"),
                fraction: 1
            )
        default:
            return MaterializationStage(stage: stage, detail: stage, fraction: nil)
        }
    }

    private func readSummary(_ payload: [String: Any]) -> MaterializationStage {
        MaterializationStage(
            stage: "summary",
            detail: "\(Self.int(payload["poem_count"])) 首 · 随包赏析 \(Self.int(payload["shipped_records"])) 条",
            fraction: 1
        )
    }

    private static func describeModelStage(_ payload: [String: Any]?) -> String {
        switch payload?["stage"] as? String {
        case "downloading":
            let done = int(payload?["bytes_done"])
            let total = int(payload?["bytes_total"])
            return total > 0
                ? "正在下载语音模型 · \(mib(done)) / \(mib(total))"
                : "正在下载语音模型 · 已写出 \(mib(done))"
        case "verifying":
            return "正在核对模型摘要 · \(mib(payload?["bytes"]))"
        case "verified":
            return "模型摘要一致"
        case "unpacking":
            return "正在解包语音模型"
        default:
            return "准备语音模型"
        }
    }

    // MARK: - JSON 与错误

    static func json(_ payload: [String: Any]) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: payload)) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    static func object(_ raw: String) throws -> [String: Any] {
        let parsed = try JSONSerialization.jsonObject(with: Data(raw.utf8))
        return parsed as? [String: Any] ?? [:]
    }

    /// 错误文案必须带上具体原因。
    ///
    /// `NativeError.Failure(message:)` 是 UniFFI 的扁平错误（Rust 侧 `#[uniffi(flat_error)]`），
    /// message 里才有真因；只写一句「操作失败」等于把真因丢掉——Android 侧在
    /// `UnsatisfiedLinkError`（`message` 为 nil）上栽过同一件事。
    static func describe(_ error: Error) -> String {
        if let native = error as? NativeError {
            switch native {
            case .Failure(let message):
                return message.isEmpty ? "原生层失败但未给出原因" : message
            }
        }
        return error.localizedDescription
    }

    private static func int(_ value: Any?) -> Int {
        if let number = value as? Int { return number }
        if let number = value as? Double { return Int(number) }
        if let number = value as? NSNumber { return number.intValue }
        return 0
    }

    private static func mib(_ value: Any?) -> String {
        String(format: "%.1f MiB", Double(int(value)) / (1024.0 * 1024.0))
    }

    private static func mib(_ value: Int) -> String {
        String(format: "%.1f MiB", Double(value) / (1024.0 * 1024.0))
    }
}

/// UniFFI 回调的接收端。
///
/// 回调**可能从 Rust 工作线程调用**（生成物的 `NativeEventSink` 文档原文），所以这里不做
/// 任何 UI 操作，只把事件翻成 `MaterializationStage` 交给上层；切回主线程由 ViewModel 负责。
private final class MaterializationSink: NativeEventSink, @unchecked Sendable {
    private let onStage: (MaterializationStage) -> Void
    private let onDone: (String?) -> Void
    private let readStage: ([String: Any]) -> MaterializationStage?
    private let readSummary: ([String: Any]) -> MaterializationStage?
    private let lock = NSLock()
    private var retained: NativeOperation?

    init(
        onStage: @escaping (MaterializationStage) -> Void,
        onDone: @escaping (String?) -> Void,
        readStage: @escaping ([String: Any]) -> MaterializationStage?,
        readSummary: @escaping ([String: Any]) -> MaterializationStage?
    ) {
        self.onStage = onStage
        self.onDone = onDone
        self.readStage = readStage
        self.readSummary = readSummary
    }

    func retain(operation: NativeOperation) {
        lock.lock()
        retained = operation
        lock.unlock()
    }

    func onEvent(eventJson: String) {
        guard let event = try? YunjianRepository.object(eventJson) else { return }
        let payload = event["payload"] as? [String: Any] ?? [:]
        switch event["type"] as? String {
        case "progress":
            readStage(payload).map(onStage)
        case "item":
            readSummary(payload).map(onStage)
        case "done":
            release()
            onDone(nil)
        case "failed":
            // `Event` 是**邻接标签**（`#[serde(tag="type", content="payload")]`），所以
            // `Failed { message }` 的 message 在 `payload` 里，不在顶层。读顶层拿到空串，
            // 于是真因被兜底文案顶掉——Android 与桌面各栽过一次同一层标签陷阱。
            let message = payload["message"] as? String ?? ""
            release()
            onDone(message.isEmpty ? "语料物化失败（后端未给出原因）" : message)
        case "cancelled":
            release()
            onDone("语料物化已取消")
        default:
            break
        }
    }

    private func release() {
        lock.lock()
        retained = nil
        lock.unlock()
    }
}

/// 首启物化在**进程内**的唯一状态。
///
/// # 为什么守卫不能挂在 ViewModel 上
///
/// 与 Android 逐字同一理由：SwiftUI 的 `@StateObject` 在场景重建（后台回收后返回、
/// XCUITest 逐条测试各拉一次界面）时会新建 ViewModel，状态回到 `idle`。于是「已经在跑」
/// 这件事在下一个 ViewModel 眼里不存在，第二次物化启动，与上一次仍在写 corpus.db 的
/// Rust 线程撞成 `database is locked`。Android 真机实测那是一个**竞态**：有些轮成功、
/// 有些轮 locked，而报错文字完全不提「有两次物化」。
///
/// **同时要回放。** 只是「第二次直接返回」会让重建后的界面停在「尚未下载语料库」，
/// 而后台其实正在解压——那句陈旧的话比空白更糟。
private final class Materialization: @unchecked Sendable {
    private let lock = NSLock()
    private var started = false
    private var finished = false
    private var failure: String?
    private var lastStage: MaterializationStage?
    private var listeners: [(stage: (MaterializationStage) -> Void, done: (String?) -> Void)] = []

    /// 登记订阅者并回放已知状态；返回 `true` 表示**本次调用**要真的去跑。
    func claim(
        onStage: @escaping (MaterializationStage) -> Void,
        onDone: @escaping (String?) -> Void
    ) -> Bool {
        lock.lock()
        if !finished {
            listeners.append((onStage, onDone))
        }
        let replayStage = lastStage
        let replayTerminal = finished
        let replayFailure = failure
        let shouldRun = !started
        started = true
        lock.unlock()

        // 回调在锁外调用：订阅者可能同步回读状态，持锁调用会自锁。
        replayStage.map(onStage)
        if replayTerminal { onDone(replayFailure) }
        return shouldRun
    }

    func publish(stage: MaterializationStage) {
        lock.lock()
        lastStage = stage
        let snapshot = listeners
        lock.unlock()
        snapshot.forEach { $0.stage(stage) }
    }

    func publish(done reason: String?) {
        lock.lock()
        finished = true
        failure = reason
        let snapshot = listeners
        listeners.removeAll()
        lock.unlock()
        snapshot.forEach { $0.done(reason) }
    }

    // **失败在本进程内是终态，刻意不自动重试。** 自动重试会让「每次场景重建都再试一次」，
    // 而上一次可能仍在跑——那正是本类要消除的并发。一次网络抖动导致后续断言报
    // 「语料不可用」是如实上报，不是需要被掩盖的东西。要重试就重启进程。
}
