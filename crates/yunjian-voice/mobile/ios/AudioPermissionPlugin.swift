import AVFoundation
import Tauri
import UIKit
import WebKit

/// iOS 麦克风授权 + `AVAudioSession` 激活。
///
/// **激活会话是必需的，不是优化。** 实读 `cpal 0.18.1` 的 iOS 后端
/// （`src/host/coreaudio/ios/mod.rs`）确认：它会 `AVAudioSession.sharedInstance()`
/// 并设 `setPreferredIOBufferDuration`，但**从不调用 `setCategory` 或 `setActive`**。
/// 而它判断设备有无输入的依据是 `inputNumberOfChannels()`，那个值在会话未激活、
/// 类别仍是默认 `.soloAmbient` 时是 0 —— 于是 `cpal` 会报告「没有输入设备」，
/// 表现为一个既不报错也录不到东西的通路。
class AudioPermissionPlugin: Plugin {

    /// 与 Rust 侧 `PermissionState` 逐项对应。改这里必须同时改 `permission.rs`。
    private static let granted = "granted"
    private static let denied = "denied"
    private static let undetermined = "undetermined"

    @objc public func checkMicrophonePermission(_ invoke: Invoke) {
        invoke.resolve(["microphone": Self.currentState()])
    }

    /// 申请授权并激活会话。顺序不可交换：未获授权时激活录音类别在部分系统版本上
    /// 直接抛错，因此先拿授权，再激活。
    @objc public func requestMicrophonePermission(_ invoke: Invoke) {
        Self.requestRecordPermission { allowed in
            guard allowed else {
                invoke.resolve(["microphone": Self.denied, "sessionActive": false])
                return
            }
            do {
                try Self.activateSession()
                invoke.resolve(["microphone": Self.granted, "sessionActive": true])
            } catch {
                // 授权拿到了但会话激活失败（例如被来电独占）。这两件事必须分开上报，
                // 否则调用方会把一次临时占用误判成用户拒绝，然后去引导改系统设置。
                invoke.resolve([
                    "microphone": Self.granted,
                    "sessionActive": false,
                    "sessionError": error.localizedDescription,
                ])
            }
        }
    }

    /// 显式停用会话，把音频焦点还给别的应用。练习结束时调用。
    @objc public func deactivateSession(_ invoke: Invoke) {
        do {
            try AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            invoke.resolve(["sessionActive": false])
        } catch {
            invoke.reject(error.localizedDescription)
        }
    }

    private static func activateSession() throws {
        let session = AVAudioSession.sharedInstance()
        // `.playAndRecord` 而不是 `.record`：朗读示范与语音默写在同一个练习界面里交替，
        // 用 `.record` 会在播放时把输出路由掐掉。
        // `.measurement` 模式关掉系统的自动增益与降噪——那些处理对识别有害，
        // 而识别器要的是尽量原始的 16 kHz 单声道。
        try session.setCategory(
            .playAndRecord,
            mode: .measurement,
            options: [.defaultToSpeaker, .allowBluetooth]
        )
        // 只是「偏好」：系统可能给别的值，因此 Rust 侧仍然无条件重采样到 16 kHz。
        try session.setPreferredSampleRate(16_000)
        try session.setPreferredInputNumberOfChannels(1)
        try session.setActive(true, options: .notifyOthersOnDeactivation)
    }

    private static func currentState() -> String {
        if #available(iOS 17.0, *) {
            switch AVAudioApplication.shared.recordPermission {
            case .granted: return granted
            case .denied: return denied
            case .undetermined: return undetermined
            @unknown default: return undetermined
            }
        }
        switch AVAudioSession.sharedInstance().recordPermission {
        case .granted: return granted
        case .denied: return denied
        case .undetermined: return undetermined
        @unknown default: return undetermined
        }
    }

    private static func requestRecordPermission(_ completion: @escaping (Bool) -> Void) {
        if #available(iOS 17.0, *) {
            AVAudioApplication.requestRecordPermission(completionHandler: completion)
        } else {
            AVAudioSession.sharedInstance().requestRecordPermission(completion)
        }
    }
}

@_cdecl("init_plugin_audio_permission")
func initPlugin() -> Plugin {
    return AudioPermissionPlugin()
}
