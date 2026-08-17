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
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
enum VoiceCapture {
    static let sampleRate: Double = 16_000
    /// 每帧 1600 采样 = 100 ms @ 16 kHz。与 Android 的 `FRAME_SAMPLES` 相同。
    static let frameSamples: Int = 1_600
    /// 一轮推送多少帧。30 帧 ≈ 3 秒，与 Android 的 `FRAMES_PER_ROUND` 相同。
    static let framesPerRound: Int = 30
    /// 静音探测读几帧。3 帧 ≈ 300 ms，足够区分全零流与真采集（与 Android 相同）。
    static let silenceProbeFrames: Int = 3

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
            try session.setCategory(.record, mode: .measurement, options: [])
            try session.setActive(true, options: [])
        } catch {
            return "音频采集不可用：AVAudioSession 无法激活（\(error.localizedDescription)）；已切到打字背诵"
        }
        guard !session.availableInputs.isNullOrEmpty else {
            return "音频采集不可用：当前没有可用输入设备；已切到打字背诵"
        }

        let engine = AVAudioEngine()
        var read = 0
        var nonZero = 0
        let gate = DispatchSemaphore(value: 0)
        let counter = SampleCounter()

        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0 else {
            return "音频采集不可用：输入格式采样率为 0，说明未取得输入路由；已切到打字背诵"
        }
        input.installTap(onBus: 0, bufferSize: AVAudioFrameCount(frameSamples), format: format) { buffer, _ in
            let (frames, nonZeroFrames) = Self.inspect(buffer)
            if counter.add(read: frames, nonZero: nonZeroFrames) >= silenceProbeFrames {
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
        _ = gate.wait(timeout: .now() + 2)
        let snapshot = counter.snapshot()
        read = snapshot.read
        nonZero = snapshot.nonZero

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
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let inputFormat = input.outputFormat(forBus: 0)
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

        let queue = FrameQueue()
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
        let deadline = Date().addingTimeInterval(15)
        while pushed < framesPerRound, Date() < deadline {
            guard let frame = queue.take() else {
                Thread.sleep(forTimeInterval: 0.02)
                continue
            }
            try push(frame)
            pushed += 1
        }
        return pushed
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

enum VoiceCaptureError: Error {
    case unsupportedFormat
}

/// tap 回调在音频线程上跑，计数必须加锁。
private final class SampleCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var read = 0
    private var nonZero = 0
    private var frames = 0

    /// 累加并返回已收到的**回调次数**（不是采样数）。
    func add(read newRead: Int, nonZero newNonZero: Int) -> Int {
        lock.lock()
        defer { lock.unlock() }
        read += newRead
        nonZero += newNonZero
        frames += 1
        return frames
    }

    func snapshot() -> (read: Int, nonZero: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (read, nonZero)
    }
}

/// 音频线程与推送线程之间的帧队列。
private final class FrameQueue: @unchecked Sendable {
    private let lock = NSLock()
    private var frames: [[Float]] = []

    func append(_ frame: [Float]) {
        guard !frame.isEmpty else { return }
        lock.lock()
        frames.append(frame)
        lock.unlock()
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
