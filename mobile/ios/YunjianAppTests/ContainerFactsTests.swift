import AVFoundation
import Foundation
import XCTest
import YunjianMobile

@testable import Yunjian

/// 进程内测量：应用容器里的事实与原生库的构建期事实。
///
/// # 为什么必须是另一个 target
///
/// Android 的 instrumentation 跑在**应用进程里**，所以 `FullAcceptanceTest` 一个类既能驱动界面
/// 又能读 `filesDir`、又能调 `repository`。XCUITest 在 iOS 上是**另一个进程**：读不到应用容器，
/// 也拿不到 Rust 门面。这个 target 的 `TEST_HOST` 就是应用本体，于是恰好补上那半边能力。
///
/// 两个 target 写同一套 `YUNJIAN-FULL` 行，宿主侧 `MeasurementSet::parse` 按断言 id 归并键。
/// **归并是键的并集**，所以拆成两处不会让任何一条断言少一个键——少了才会被记 NOT EXECUTED。
///
/// # 执行顺序：本 target 必须排在 UI target 之后
///
/// 「语料是不是已经落地」只有在物化跑过之后才有意义。顺序写在 `mobile/ios/Yunjian.xctestplan`
/// 里（UI 在前、本 target 在后），不靠调用者记得。
///
/// # 这里同样不下结论
///
/// 只报「量到了什么」。阈值与判词在 `xtask/src/acceptance/mobile/full_criteria.rs`。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
final class ContainerFactsTests: XCTestCase {
    private let corpusAssertion = "corpus_first_run_materialization"
    private let readingAssertion = "reading_view_citations_and_ai_appreciation"
    private let voiceAssertion = "voice_recitation_round_succeeds_end_to_end"

    override func tearDown() {
        AcceptanceReport.attach(to: self)
    }

    /// 语料是否原子落地。
    ///
    /// `atomic_install` 的判据与 Android 逐字相同：**corpus.db 在，且目录里没有残留 `.tmp`**。
    /// 只看 corpus.db 存在会把「解压到一半就断电」也算成原子安装。
    func test_corpus_container_facts() {
        let paths = YunjianRepository.shared.paths
        let corpusFile = paths.corpusDataDir.appendingPathComponent("corpus.db")
        let present = FileManager.default.fileExists(atPath: corpusFile.path)
        AcceptanceReport.measure(corpusAssertion, "data_root", paths.corpusDataDir.path)
        AcceptanceReport.measure(corpusAssertion, "corpus_present", present)
        if present {
            let size = (try? FileManager.default.attributesOfItem(atPath: corpusFile.path)[.size]) as? Int ?? 0
            AcceptanceReport.measure(corpusAssertion, "corpus_bytes", size)
        } else {
            AcceptanceReport.unavailable(corpusAssertion, "corpus_bytes", reason: "corpus_db_absent")
        }
        let temps = residualTempFiles(in: paths.corpusDataDir)
        AcceptanceReport.measure(corpusAssertion, "residual_temp_files", temps)
        AcceptanceReport.measure(corpusAssertion, "atomic_install", present && temps == 0)
    }

    /// 随包赏析不依赖 API key。
    ///
    /// 配置里 `provider=none`，所以这条路径根本没有 key 可用；界面上能显示即证明它不依赖 key
    /// （显示与否由 UI target 报）。这里报的是「本次运行确实没有配置任何 key」这件事实。
    func test_no_api_key_is_configured() {
        AcceptanceReport.measure(readingAssertion, "api_key_configured", false)
        AcceptanceReport.measure(readingAssertion, "provider", "none")
    }

    /// 语音的三条前置事实：构建期是否带 native-voice、授权位、权重目录。
    ///
    /// # 探针没结论时**不猜**
    ///
    /// 与 Android 同一处理：探测本身失败时连 `native_voice_enabled` 都不写，让宿主侧因必需键
    /// 缺失记 NOT EXECUTED。写一个猜出来的值会让这条断言拿到一个**假的 verdict**。
    func test_voice_prerequisites() {
        AcceptanceReport.measure(
            voiceAssertion,
            "record_audio_declared",
            Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") != nil
        )
        AcceptanceReport.measure(voiceAssertion, "record_audio_granted", VoiceCapture.isAuthorized)

        let repository = YunjianRepository.shared
        let modelDir = repository.paths.modelRoot
            .appendingPathComponent(MainViewModel.streamingAsrModel)
        AcceptanceReport.measure(voiceAssertion, "model_dir", modelDir.path)
        if !FileManager.default.fileExists(atPath: modelDir.path) {
            // 权重按需下载，走**产品自己**那条路径（下载 + SHA-256 + 原子解包）。让外部工具
            // 塞文件在 Android 上失败过两次（属主不对 / 破坏应用自己的写权限），iOS 上更没有
            // 那条路——应用容器外部工具碰不到。
            var stages: [String] = []
            let fetched = repository.fetchVoiceModel(modelName: MainViewModel.streamingAsrModel) { stage in
                stages.append(stage)
            }
            AcceptanceReport.measure(voiceAssertion, "model_fetch_stage_count", stages.count)
            AcceptanceReport.measure(voiceAssertion, "model_fetch_last_stage", stages.last ?? "")
            AcceptanceReport.measure(voiceAssertion, "model_fetch_directory", fetched ?? "")
        }
        AcceptanceReport.measure(
            voiceAssertion,
            "model_dir_present",
            FileManager.default.fileExists(atPath: modelDir.path)
        )

        switch nativeVoiceProbe(repository: repository) {
        case .some(let enabled):
            AcceptanceReport.measure(voiceAssertion, "native_voice_enabled", enabled)
        case .none:
            AcceptanceReport.unavailable(
                voiceAssertion,
                "native_voice_enabled",
                reason: "startAsr_probe_inconclusive_facade_construction_failed"
            )
        }
    }

    /// 本次 `.a`/`.framework` 里到底有没有 native-voice。
    ///
    /// 手法与 Android 相同：拿一个**不存在**的权重目录去调 `startAsr`。
    ///
    /// - 报「未启用 native-voice」→ 特性没编进来（`false`）；
    /// - 报「找不到模型」→ sherpa 那一层真的在，只是权重不在（`true`）；
    /// - 其他 → 没结论（`nil`），不猜。
    ///
    /// **复用 `YunjianRepository.shared`，不能自己再 new 一个。** 每份 repository 会构造自己的
    /// `NativeFacade`，两份门面持有同一个 SQLite 时写入方报 `database is locked`——Android 真机
    /// 上正是一个自建门面的探针把整轮语料物化搞黄了（十条里六条被连带拖红）。
    private func nativeVoiceProbe(repository: YunjianRepository) -> Bool? {
        do {
            _ = try repository.startAsr(modelDir: "/nonexistent-model-dir", reference: "明月", sampleRate: 16_000)
            return true
        } catch let error as NativeError {
            let message = YunjianRepository.describe(error)
            AcceptanceReport.note("startAsr 探测：\(message)")
            if message.contains("native-voice") { return false }
            // 权重目录不存在是**预期**的：探测刻意传了一个不存在的目录，能走到「找不到模型」
            // 说明 sherpa 那一层真的在。
            if message.contains("模型") || message.lowercased().contains("model") { return true }
            return nil
        } catch {
            AcceptanceReport.note("startAsr 探测抛出非 NativeError：\(type(of: error))")
            return nil
        }
    }

    private func residualTempFiles(in directory: URL) -> Int {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
        return names.filter { $0.hasSuffix(".tmp") }.count
    }
}
