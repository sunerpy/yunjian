import AVFoundation
import Foundation

/// 麦克风采集。Android 用 `AudioRecord`，iOS 用 `AVAudioEngine`；**两侧的判据完全一样**。
///
/// # 为什么要单独一层
///
/// 语音一轮的产品行为有三条降级路径（未授权、权重目录不在、采集拿不到数据），其中第三条
/// 在 Android 真机上逮到过一个真缺陷：`checkSelfPermission` 报已授予，但 appops 拒了
/// `android:record_audio`，于是 `AudioRecord` 读到**静音流**——产品会静静录一段空白，
/// 用户看到「正在采集」却永远没有结果，比明确降级更糟。
///
/// iOS 上有形态相同的状态：`AVAudioSession` 被其他应用占用、`recordPermission` 为
/// `.granted` 但输入路由为空（无输入设备）、或系统级麦克风被静音时，tap 回调照样触发而
/// 采样全零。所以这里保留 Android 那条判据：**这一小段里有没有任何非零采样**。
/// 用能量阈值反而会把安静房间里的真采集误判成被拒。
///
/// # 时长按采样率换算，不按回调次数
///
/// iOS 的 tap 按**设备**输入格式交付，而 `installTap` 的 `bufferSize` 在 Apple 文档里只是建议值
/// （“The size of the incoming buffers. The implementation may choose another size.”）。所以
/// 「读了 N 次回调」与「过了多少毫秒」之间没有固定换算：在常见的 44.1/48 kHz 设备上按 16 kHz
/// 的帧长数 30 次回调，实际只有约 1 秒。本文件里凡是「一轮多长」「探测多长」都以**秒**声明，
/// 帧数由采样率算出来；重采样只改采样率不改源时长，补不回缺掉的两秒。
///
/// # AVAudioSession 在所有出口停用
///
/// 激活与停用严格配对，停用一律写在 `defer` 里——降级路径全是提前 `return`，写在函数末尾时
/// 它们会把录音 session 留在激活状态，而后果落在**别的应用**上，本应用自己的验收永远量不到。
/// 停用排在停 engine 之后（`defer` 后进先出）：对还有音频对象在跑的 session 调停用会拿到
/// `isBusy`。两条都由 `xtask` 的 `verify_voice_capture_contract` 守住。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。用到的 AVFoundation API 名与签名逐个
/// 核对过 Apple 官方文档，但**编译与真机行为未验证**。见 `mobile/ios/README.md`。
enum VoiceCapture {
    static let sampleRate: Double = 16_000
    /// 每帧 1600 采样 = 100 ms @ 16 kHz。与 Android 的 `FRAME_SAMPLES` 相同。
    ///
    /// 这个等式成立的前提是**帧在重采样之后按这个长度切分**（见 `FrameQueue`），而不是把 tap
    /// 一次回调交付的缓冲当成一帧。
    static let frameSamples: Int = 1_600
    /// 一轮采集的目标时长。与 Android 的 `FRAMES_PER_ROUND × FRAME_SAMPLES ÷ SAMPLE_RATE`
    /// 是同一个 3.0 秒，由 `xtask` 的 `verify_round_duration_parity` 逐条比对。
    static let roundSeconds: Double = 3
    /// 一轮推多少帧 = 目标时长 × 采样率 ÷ 每帧采样数。
    ///
    /// **刻意不写成字面量 30**：那个数字只在 16 kHz、每帧 1600 采样这一种组合下等于 3 秒。
    static var framesPerRound: Int { Int((roundSeconds * sampleRate).rounded()) / frameSamples }
    /// 一轮的墙钟上限：目标时长再加启动与调度余量。到点仍不足帧数时按实际推送数返回。
    static var roundDeadlineSeconds: Double { roundSeconds + 12 }
    /// 静音探测读多长。300 ms 足够区分全零流与真采集（与 Android 的 3 帧 × 100 ms 相同）。
    ///
    /// 按**时长**而不是回调次数记：tap 的 `bufferSize` 在 Apple 文档里只是建议值
    /// （“The implementation may choose another size.”），回调次数与时长没有固定换算。
    static let silenceProbeSeconds: Double = 0.3
    /// 等静音探测读满的墙钟上限。超时不是「被拒」而是「读不到任何采样」。
    static let probeTimeoutSeconds: Double = 2

    /// 当前的录音授权状态。
    ///
    /// 只读不问：请求授权是一次会弹窗的动作，放在这里会让「探测状态」产生副作用，
    /// 而 XCUITest 里弹窗需要 interruption monitor 才能点掉——探测与请求必须分开。
    static var isAuthorized: Bool {
        if #available(iOS 17.0, *) {
            return AVAudioApplication.shared.recordPermission == .granted
        }
        return AVAudioSession.sharedInstance().recordPermission == .granted
    }

    /// 采集能否真的拿到非静音数据。可用时返回 `nil`，否则返回可直接展示的原因。
    ///
    /// 与 Android 的 `probeSilentCapture` 一一对应，包括「读不到任何采样」与「读到了但全零」
    /// 两条**分开**的原因——它们指向不同的处置（前者是路由/占用，后者是权限或系统静音）。
    static func probeSilentCapture() -> String? {
        let session = AVAudioSession.sharedInstance()
        do {
            try activateRecordingSession()
        } catch {
            return "音频采集不可用：AVAudioSession 无法激活（\(error.localizedDescription)）；已切到打字背诵"
        }
        // 停用必须在 `defer` 里：下面每一条降级 `return` 都是提前出口，写在函数末尾时它们
        // 会把已激活的录音 session 留在系统里。
        defer { deactivateRecordingSession() }

        guard !session.availableInputs.isNullOrEmpty else {
            return "音频采集不可用：当前没有可用输入设备；已切到打字背诵"
        }

        let engine = AVAudioEngine()
        let gate = DispatchSemaphore(value: 0)
        let counter = SampleCounter()

        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0 else {
            return "音频采集不可用：输入格式采样率为 0，说明未取得输入路由；已切到打字背诵"
        }
        // 探测读满多少采样按**设备**采样率算：tap 交付的是设备格式，用 16 kHz 下的数字会在
        // 48 kHz 设备上只读到三分之一的时长。
        let probeSamples = Int((silenceProbeSeconds * format.sampleRate).rounded())
        input.installTap(onBus: 0, bufferSize: AVAudioFrameCount(frameSamples), format: format) { buffer, _ in
            let (frames, nonZeroFrames) = Self.inspect(buffer)
            if counter.add(read: frames, nonZero: nonZeroFrames).read >= probeSamples {
                gate.signal()
            }
        }
        defer {
            input.removeTap(onBus: 0)
            engine.stop()
        }
        do {
            try engine.start()
        } catch {
            return "音频采集被拒：AVAudioEngine 启动失败（\(error.localizedDescription)）；已切到打字背诵"
        }
        // 等一小段。超时不是「被拒」而是「读不到任何采样」，两者的原因文字必须不同。
        _ = gate.wait(timeout: .now() + probeTimeoutSeconds)
        let snapshot = counter.snapshot()
        let read = snapshot.read
        let nonZero = snapshot.nonZero

        if read == 0 {
            return "音频采集被拒：读不到任何采样（麦克风可能被其他应用占用或无输入路由）；已切到打字背诵"
        }
        if nonZero == 0 {
            return "音频采集被拒：\(read) 个采样全为静音，麦克风授权或系统输入未真正放开；已切到打字背诵"
        }
        return nil
    }

    /// 逐帧读取一轮，把 16 kHz 单声道 Float 采样交给 `push`。
    ///
    /// 返回实际推送的帧数。**不在这里做任何判定**：帧数是测量值，够不够由上层与宿主侧判据说。
    static func pushRound(push: ([Float]) throws -> Void) throws -> Int {
        try activateRecordingSession()
        // 注册顺序即执行顺序的反序：这条先注册所以**最后**执行，于是 session 在 engine 停下
        // 之后才停用。反过来的话，对还有音频对象在跑的 session 调停用会拿到
        // `AVAudioSession.ErrorCode.isBusy`（Apple 文档 `setActive(_:options:)` 明载），
        // session 反而留在激活状态。
        defer { deactivateRecordingSession() }

        let engine = AVAudioEngine()
        let input = engine.inputNode
        let inputFormat = input.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0 else {
            throw VoiceCaptureError.noInputRoute
        }
        guard let target = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: sampleRate,
            channels: 1,
            interleaved: false
        ) else {
            throw VoiceCaptureError.unsupportedFormat
        }
        guard let converter = AVAudioConverter(from: inputFormat, to: target) else {
            throw VoiceCaptureError.unsupportedFormat
        }

        // 队列按 16 kHz 下的 `frameSamples` 切分**重采样之后**的流，所以一帧恒等于 100 ms。
        // 旧写法把 tap 的一次回调当成一帧，而 tap 按设备采样率交付、`bufferSize` 在 Apple
        // 文档里只是建议值（“The implementation may choose another size.”），于是 48 kHz 上
        // 一帧只有约 33 ms，一轮 30 帧约等于 1 秒——「30 帧 ≈ 3 秒」在真机上根本不成立。
        let queue = FrameQueue(frameSamples: frameSamples)
        input.installTap(onBus: 0, bufferSize: AVAudioFrameCount(frameSamples), format: inputFormat) { buffer, _ in
            // 重采样到 16 kHz 单声道：设备输入常见 44.1/48 kHz，直接把原始采样送进 ASR
            // 会让识别在时间轴上整体拉伸，而报错形态是「识别结果对不上」——那不指向真因。
            guard let converted = Self.convert(buffer, using: converter, to: target) else { return }
            queue.append(Self.samples(of: converted))
        }
        defer {
            input.removeTap(onBus: 0)
            engine.stop()
        }
        try engine.start()

        var pushed = 0
        let targetFrames = framesPerRound
        let deadline = Date().addingTimeInterval(roundDeadlineSeconds)
        while pushed < targetFrames, Date() < deadline {
            guard let frame = queue.take() else {
                Thread.sleep(forTimeInterval: 0.02)
                continue
            }
            try push(frame)
            pushed += 1
        }
        return pushed
    }

    /// 激活录音 session。**与 `deactivateRecordingSession` 严格配对**，由 `xtask` 的
    /// `verify_session_is_deactivated` 按调用点计数守住。
    private static func activateRecordingSession() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .measurement, options: [])
        try session.setActive(true, options: [])
    }

    /// 停用录音 session，并通知被打断的其他应用可以恢复。
    ///
    /// 不 `throws`：它只在 `defer` 里被调用，而从 `defer` 里抛错会盖掉真正的失败原因。
    /// 停用失败本身不是产品缺陷（最常见的是 `isBusy`），但漏掉停用是——那会让别的应用
    /// 在本应用录完一轮之后拿不回音频。
    private static func deactivateRecordingSession() {
        try? AVAudioSession.sharedInstance().setActive(false, options: [.notifyOthersOnDeactivation])
    }

    private static func convert(
        _ buffer: AVAudioPCMBuffer,
        using converter: AVAudioConverter,
        to format: AVAudioFormat
    ) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 1
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var consumed = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, status in
            if consumed {
                status.pointee = .noDataNow
                return nil
            }
            consumed = true
            status.pointee = .haveData
            return buffer
        }
        return error == nil ? output : nil
    }

    private static func samples(of buffer: AVAudioPCMBuffer) -> [Float] {
        guard let channel = buffer.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength)))
    }

    private static func inspect(_ buffer: AVAudioPCMBuffer) -> (frames: Int, nonZero: Int) {
        let values = samples(of: buffer)
        if values.isEmpty {
            // Int16 输入格式下 `floatChannelData` 为 nil；那时按「读到了但读不出数值」处理，
            // 而不是当成静音——两者的处置不同。
            return (Int(buffer.frameLength), Int(buffer.frameLength))
        }
        return (values.count, values.reduce(into: 0) { $0 += ($1 == 0 ? 0 : 1) })
    }
}

/// 采集失败的原因。
///
/// conform `LocalizedError` 是必需的：`MainViewModel` 把 `describe(error)` 的结果直接贴到
/// 降级文案里，而裸 `Error` 的 `localizedDescription` 会给出
/// 「The operation couldn't be completed. (VoiceCaptureError error 0.)」——那不指向真因。
enum VoiceCaptureError: LocalizedError {
    case unsupportedFormat
    case noInputRoute

    var errorDescription: String? {
        switch self {
        case .unsupportedFormat:
            return "音频采集不可用：无法建立 16 kHz 单声道重采样通路"
        case .noInputRoute:
            return "音频采集不可用：输入格式采样率为 0，说明未取得输入路由"
        }
    }
}

/// tap 回调在音频线程上跑，计数必须加锁。
private final class SampleCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var read = 0
    private var nonZero = 0

    /// 累加并返回累计**采样数**。
    ///
    /// 刻意不返回回调次数：tap 的 `bufferSize` 只是建议值，回调次数与时长没有固定换算，
    /// 按次数判「读够了」会随设备采样率漂移。
    func add(read newRead: Int, nonZero newNonZero: Int) -> (read: Int, nonZero: Int) {
        lock.lock()
        defer { lock.unlock() }
        read += newRead
        nonZero += newNonZero
        return (read, nonZero)
    }

    func snapshot() -> (read: Int, nonZero: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (read, nonZero)
    }
}

/// 音频线程与推送线程之间的帧队列，按 `frameSamples` **重新切分**输入。
///
/// 切分是这个类存在的理由。tap 交付的缓冲长度由系统决定（`bufferSize` 只是建议值），重采样
/// 之后长度还会再变；直接把每次交付当成一帧会让「一帧 = 100 ms」这个换算失效，而失效的表现
/// 是一轮的真实时长只有声称的三分之一——`total_ms` 与「停顿」读数全都跟着偏，却没有任何报错。
private final class FrameQueue: @unchecked Sendable {
    private let lock = NSLock()
    private let frameSamples: Int
    private var pending: [Float] = []
    private var frames: [[Float]] = []

    init(frameSamples: Int) {
        self.frameSamples = frameSamples
    }

    func append(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        lock.lock()
        defer { lock.unlock() }
        pending.append(contentsOf: samples)
        while pending.count >= frameSamples {
            frames.append(Array(pending.prefix(frameSamples)))
            pending.removeFirst(frameSamples)
        }
    }

    func take() -> [Float]? {
        lock.lock()
        defer { lock.unlock() }
        return frames.isEmpty ? nil : frames.removeFirst()
    }
}

private extension Optional where Wrapped == [AVAudioSessionPortDescription] {
    var isNullOrEmpty: Bool {
        switch self {
        case .none: return true
        case .some(let ports): return ports.isEmpty
        }
    }
}
