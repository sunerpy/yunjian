import Foundation
import YunjianMobile

/// 首启物化的四态。与 Android 的 `CorpusState` 一一对应。
enum CorpusState: Equatable {
    case idle
    case working(MaterializationStage)
    case ready(facts: String)
    case failed(reason: String)
}

/// 语音一轮的可观测状态。与 Android 的 `VoiceState` 一一对应。
enum VoiceState: Equatable {
    case idle
    case listening(detail: String)
    case finished(detail: String)
    /// 语音不可用，已降级到打字。
    ///
    /// `reason` 必须是**具体原因**而不是「语音不可用」：验收判据检的正是
    /// `reason_names_capture_denial`，一句笼统的话与不显示等价。
    case degraded(reason: String)
}

/// 界面状态。字段与 Android 的 `UiState` 一一对应，顺序也一样——两侧的断言读同一批东西。
struct UiState: Equatable {
    var corpus: CorpusState = .idle
    var query: String = ""
    var directId: String = ""
    var hits: [SearchHit] = []
    var searched: Bool = false
    var reading: PoemReading?
    var reciteSession: ReciteSession?
    var reciteAnswer: String = ""
    var reciteScore: ReciteScore?
    var voice: VoiceState = .idle
    var error: String?
}

/// 全部界面行为。Android 是 `MainViewModel`，这里是同名同职责的 `ObservableObject`。
///
/// # 为什么状态集中在一个 `@Published`
///
/// Android 侧用单个 `MutableStateFlow<UiState>`。拆成多个 `@Published` 会让「检索成功时要
/// 同时清掉上一首阅读页」这类**成组**变更出现中间帧，而那个中间帧正是真机上逮到的缺陷形态
/// （旧正文压在新结果上面，想点的按钮跑到屏幕外，点击静默落空）。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
@MainActor
final class MainViewModel: ObservableObject {
    @Published private(set) var state = UiState()

    private let repository: YunjianRepository

    /// 流式识别模型目录名。与 `models/cache/` 下的上游发布包同名，与 Android 侧相同。
    static let streamingAsrModel = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20"

    init(repository: YunjianRepository = .shared) {
        self.repository = repository
    }

    /// 权重目录。与 Android 的 `MainActivity` 同一口径：应用容器内的模型根 + 模型名。
    var modelDir: String {
        repository.paths.modelRoot.appendingPathComponent(Self.streamingAsrModel).path
    }

    /// 首启物化。
    ///
    /// 语料已在本地时**也走这条路径**：`AssetResolver` 的 `already_present` 分支会立刻报出来，
    /// 界面因此总有话说。跳过它会让「第二次启动界面一片空白」成为一个正常状态。
    ///
    /// 三态都要挡：`working` 是正在跑，`ready` 是已经跑完。只挡 `working` 时场景重建会在
    /// 已就绪之后再跑一次，而那一次与仍持有句柄的门面撞成 `database is locked`
    /// （Android 真机实测）。
    func materialize() {
        if case .working = state.corpus { return }
        if case .ready = state.corpus { return }
        state.corpus = .working(MaterializationStage(stage: "starting", detail: "正在联系发布地址", fraction: nil))
        repository.materialize(
            onStage: { [weak self] stage in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if stage.stage == "summary" {
                        self.state.corpus = .ready(facts: stage.detail)
                    } else {
                        self.state.corpus = .working(stage)
                    }
                }
            },
            onDone: { [weak self] failure in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if let failure {
                        self.state.corpus = .failed(reason: failure)
                        return
                    }
                    if case .ready = self.state.corpus { return }
                    self.state.corpus = .ready(facts: self.readFacts())
                }
            }
        )
    }

    private func readFacts() -> String {
        do {
            return try repository.corpusStatusJson()
        } catch {
            return "语料状态不可读：\(YunjianRepository.describe(error))"
        }
    }

    func onQueryChange(_ value: String) { state.query = value }

    func onDirectIdChange(_ value: String) { state.directId = value }

    /// 发起一次检索。
    ///
    /// **新的检索会收掉上一首的阅读页。** 阅读页独占一屏（见 `ContentView`），不收掉它，
    /// 用户重新检索后看到的仍是上一首——Android 真机上这条曾让 `performClick` 静默落空，
    /// 并把一次装置问题记成产品 FAIL。这是产品缺陷而不是断言问题：真人遇到的是同一件事。
    func search() {
        let query = state.query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        run {
            let hits = try self.repository.searchText(query: query)
            return { state in
                state.hits = hits
                state.searched = true
                state.error = nil
                state.reading = nil
            }
        } onFailure: { message in
            { state in
                state.error = message
                state.searched = true
            }
        }
    }

    func openReading(poemId: String) {
        let trimmed = poemId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        run {
            let reading = try self.repository.reading(poemId: trimmed)
            return { state in
                state.reading = reading
                state.error = nil
            }
        }
    }

    /// 关掉阅读页回到检索。阅读页独占一屏，这是唯一的出口。
    func closeReading() { state.reading = nil }

    func startRecite(poemId: String) {
        run {
            let session = try self.repository.reciteStart(poemId: poemId)
            return { state in
                state.reciteSession = session
                // 预填正文：「向已有内容的字段输入」这条断言要求字段一开始就不为空。
                // 空字段测不出「输入法在已有文本上追加」这件事。与 Android 同一预填值。
                state.reciteAnswer = "明月"
                state.reciteScore = nil
                state.error = nil
            }
        }
    }

    func onReciteAnswerChange(_ value: String) { state.reciteAnswer = value }

    func submitRecite(grade: String = "good") {
        guard let session = state.reciteSession else { return }
        let answer = state.reciteAnswer
        run {
            let score = try self.repository.reciteSubmit(
                poemId: session.poemId,
                answer: answer,
                grade: grade
            )
            return { state in
                state.reciteScore = score
                state.error = nil
            }
        }
    }

    /// 一轮语音跟读。
    ///
    /// 三条降级路径都写成 `.degraded` 并带具体原因，与 Android 逐条对应：
    ///
    /// 1. 未取得麦克风授权；
    /// 2. ASR 权重目录不在（模型按需下载，未下载时不是缺陷）；
    /// 3. 采集拿不到数据（授权位为真但读到静音流）。
    ///
    /// **刻意不做的事**：不把识别结果送进评分。2026-08-11 裁决按 1800 句实测 CER 77.01%
    /// 定下 `guided_practice`——只报「是否开口／停顿／相对节奏」，等级由用户自选。
    /// 判据 `auto_graded == false` 是那个裁决的可执行形态。
    func startVoiceRound(poemId: String, reference: String, modelDir: String) {
        guard VoiceCapture.isAuthorized else {
            state.voice = .degraded(reason: "未取得麦克风授权（NSMicrophoneUsageDescription 对应的权限被拒绝）；已切到打字背诵")
            startRecite(poemId: poemId)
            return
        }
        guard FileManager.default.fileExists(atPath: modelDir) else {
            state.voice = .degraded(reason: "ASR 权重目录不存在：\(modelDir)；已切到打字背诵")
            startRecite(poemId: poemId)
            return
        }
        if let silent = VoiceCapture.probeSilentCapture() {
            state.voice = .degraded(reason: silent)
            startRecite(poemId: poemId)
            return
        }
        state.voice = .listening(detail: "正在采集")
        Task.detached { [repository] in
            do {
                let detail = try Self.runVoiceRound(
                    repository: repository,
                    reference: reference,
                    modelDir: modelDir
                )
                await MainActor.run { self.state.voice = .finished(detail: detail) }
            } catch {
                let message = YunjianRepository.describe(error)
                await MainActor.run {
                    self.state.voice = .degraded(reason: "\(message)；已切到打字背诵")
                    self.startRecite(poemId: poemId)
                }
            }
        }
    }

    /// 采集 → 推送 → 拉取事件。返回可直接显示的一行测量文字。
    ///
    /// 文案与 Android 的 `describeItem` 逐字相同：`spoke=` / `pause_count=` / `total_ms=` /
    /// `single_rtf=` 四个键，验收判据从界面上读走的正是它们。改这行文案会让
    /// `voice_recitation_round_succeeds_end_to_end` 在 iOS 上永远量不到值。
    private static func runVoiceRound(
        repository: YunjianRepository,
        reference: String,
        modelDir: String
    ) throws -> String {
        let operation = try repository.startAsr(
            modelDir: modelDir,
            reference: reference,
            sampleRate: UInt32(VoiceCapture.sampleRate)
        )
        defer { operation.shutdown() }
        _ = try VoiceCapture.pushRound { frame in
            try operation.pushPcm(samples: frame)
        }
        operation.finishInput()

        var outcome = ""
        // 持续拉取直到唯一终态。一次轮询超时（`nil`）不是终态——把它当终态会让语音一轮在
        // 识别还没吐出 outcome 时就被判成结束。
        while true {
            guard let raw = operation.nextEvent(timeoutMs: 200) else { continue }
            let event = try YunjianRepository.object(raw)
            let payload = event["payload"] as? [String: Any] ?? [:]
            switch event["type"] as? String {
            case "item":
                outcome += describeItem(payload)
            case "done":
                return outcome.isEmpty ? "识别结束但未产出 outcome" : outcome
            case "failed":
                throw VoiceRoundError.failed(payload["message"] as? String ?? "语音会话失败")
            case "cancelled":
                throw VoiceRoundError.failed("语音会话已取消")
            default:
                continue
            }
        }
    }

    private static func describeItem(_ payload: [String: Any]) -> String {
        guard payload["type"] as? String == "outcome" else { return "" }
        let spoke = payload["spoke"] as? Bool ?? false
        let pauses = payload["pause_count"] as? Int ?? 0
        let totalMs = payload["total_ms"] as? Int ?? 0
        let rtf = payload["single_rtf"] as? Double ?? 0
        return "开口=\(spoke) 停顿=\(pauses) 时长=\(totalMs)ms 单路RTF=\(rtf)"
    }

    func dismissError() { state.error = nil }

    /// 把一次可能失败的原生调用搬到后台，成功与失败各自产出一个状态变更。
    ///
    /// 与 Android 的 `runCatching { withContext(Dispatchers.IO) { … } }` 同形：原生调用会打开
    /// SQLite 并可能跑数秒，放在主线程上界面会僵住，而僵住在真机上表现为「点了没反应」。
    private func run(
        _ work: @escaping () throws -> (inout UiState) -> Void,
        onFailure: @escaping (String) -> (inout UiState) -> Void = { message in { $0.error = message } }
    ) {
        Task.detached {
            let apply: (inout UiState) -> Void
            do {
                apply = try work()
            } catch {
                apply = onFailure(YunjianRepository.describe(error))
            }
            await MainActor.run { self.state.apply(apply) }
        }
    }
}

enum VoiceRoundError: Error {
    case failed(String)
}

private extension UiState {
    mutating func apply(_ change: (inout UiState) -> Void) {
        change(&self)
    }
}
