package top.yunjian.plugin.audiopermission

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * 麦克风授权。**这个类存在的唯一理由是 Android 的运行时权限对话框只能由 Android
 * framework 发起**——`cpal` 走 AAudio 采集，绕开了 WebView，但绕不开权限模型，
 * 而 NDK 侧没有任何 API 能弹出那个对话框。
 *
 * 两条权限刻意分成两个 alias 上报：
 * - `microphone` = RECORD_AUDIO，dangerous 级，用户可拒，需要运行时申请；
 * - `audioSettings` = MODIFY_AUDIO_SETTINGS，normal 级，只要 manifest 声明了就在
 *   安装时授予。把它并进同一个 alias 会让「已声明」与「已授权」在上报里无法区分，
 *   而缺声明恰好是 tauri#10846 那个坑的成因，所以它必须能被单独观测。
 */
@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = AudioPermissionPlugin.ALIAS_MICROPHONE),
        Permission(strings = [Manifest.permission.MODIFY_AUDIO_SETTINGS], alias = AudioPermissionPlugin.ALIAS_AUDIO_SETTINGS),
    ],
)
class AudioPermissionPlugin(private val activity: Activity) : Plugin(activity) {

    companion object {
        const val ALIAS_MICROPHONE = "microphone"
        const val ALIAS_AUDIO_SETTINGS = "audioSettings"

        /** 与 Rust 侧 `PermissionState` 逐项对应。改这里必须同时改 `permission.rs`。 */
        private const val STATE_GRANTED = "granted"
        private const val STATE_DENIED = "denied"
        private const val STATE_UNDETERMINED = "undetermined"
    }

    /** 只查，不弹窗。UI 用它决定是否显示「开启语音默写」入口。 */
    @Command
    fun checkMicrophonePermission(invoke: Invoke) {
        invoke.resolve(report())
    }

    /**
     * 申请。已授权时直接返回，不重复弹窗。
     *
     * 只为 RECORD_AUDIO 走运行时申请：normal 级权限传给 `requestPermissions` 会被
     * 系统忽略，把它塞进来只会让回调语义变得难以推断。
     */
    @Command
    fun requestMicrophonePermission(invoke: Invoke) {
        if (isGranted(Manifest.permission.RECORD_AUDIO)) {
            invoke.resolve(report())
            return
        }
        requestPermissionForAlias(ALIAS_MICROPHONE, invoke, "microphonePermissionCallback")
    }

    @PermissionCallback
    private fun microphonePermissionCallback(invoke: Invoke) {
        invoke.resolve(report())
    }

    private fun isGranted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED

    /**
     * 上报三件事，而不是一个 bool：录音权限状态、音频设置权限状态、以及
     * `shouldShowRequestPermissionRationale` 取反后的「永久拒绝」判定。
     *
     * 最后一项是必需的：Android 在用户勾选「不再询问」后，`requestPermissions` 会
     * **立即以拒绝回调返回且不弹窗**，此时 UI 必须引导去系统设置而不是再点一次按钮。
     */
    private fun report(): JSObject {
        val recordGranted = isGranted(Manifest.permission.RECORD_AUDIO)
        val settingsGranted = isGranted(Manifest.permission.MODIFY_AUDIO_SETTINGS)
        val canAskAgain = activity.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO)

        val microphoneState = when {
            recordGranted -> STATE_GRANTED
            canAskAgain -> STATE_UNDETERMINED
            else -> STATE_DENIED
        }

        return JSObject().apply {
            put("microphone", microphoneState)
            put("audioSettings", if (settingsGranted) STATE_GRANTED else STATE_DENIED)
            put("permanentlyDenied", !recordGranted && !canAskAgain)
            put("declaredPermissions", ARRAY_DECLARED)
        }
    }
}

/** 与 Rust 侧 `ANDROID_PERMISSIONS` 逐项对应，供上报时自证 manifest 声明齐全。 */
private val ARRAY_DECLARED = arrayOf(
    Manifest.permission.RECORD_AUDIO,
    Manifest.permission.MODIFY_AUDIO_SETTINGS,
)
