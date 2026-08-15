package top.onethinker.yunjian.spike

import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlin.math.abs
import kotlin.math.sqrt
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 判据①：在物理设备上经 todo 46 的权限路径采集 16 kHz 单声道 PCM 并算出非零 RMS。
 *
 * ## 为什么测的是「权限路径」而不是那个 Kotlin 插件类
 *
 * todo 46 的 `AudioPermissionPlugin` 是一个 **Tauri 插件**，要被 Rust 侧
 * `tauri::plugin` 注册才会存在于进程里，而那份注册属于 todo 69 的 `tauri_mobile` 分支——
 * 也就是**本门禁要决定的那件事**。让判据①依赖它就成了循环：门禁要等 binding，
 * binding 要等门禁。
 *
 * 所以这里测它的**可观测契约**，也就是插件 `report()` 返回的那三件事里可独立验证的两件：
 * 两条权限在已安装包的 manifest 中声明齐全（缺 `MODIFY_AUDIO_SETTINGS` 正是 tauri#10846
 * 那个坑），且 `RECORD_AUDIO` 在运行期确实已授予。报告里如实写明插件类未参与本轮。
 *
 * ## Device Farm 没有音频注入
 *
 * 因此非零 RMS 只能来自机架环境噪声。判据①要的正是「采到的不是静音」，这一点机架噪声
 * 足以证明；但它**不蕴含** todo 71 的 `voice_recitation_round_succeeds_end_to_end`——
 * 那条要求识别一段**已知**背诵，而已知音频喂不进这台设备。两者不可互相推导。
 */
@RunWith(AndroidJUnit4::class)
class SpikeMicrophoneTest {

    @Test
    fun captures_16khz_mono_pcm_with_non_zero_rms() {
        SpikeReport.identify(CRITERION)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext

        val declared = declaredPermissions(context)
        val recordDeclared = Manifest.permission.RECORD_AUDIO in declared
        val settingsDeclared = Manifest.permission.MODIFY_AUDIO_SETTINGS in declared
        SpikeReport.measure(CRITERION, "record_audio_declared", recordDeclared)
        SpikeReport.measure(CRITERION, "modify_audio_settings_declared", settingsDeclared)
        SpikeReport.measure(CRITERION, "permission_plugin_class_present", false)

        if (recordDeclared) {
            // `pm grant` 走 UiAutomation，避免弹出对话框；未声明的权限 grant 会抛，
            // 所以先看声明再授权。
            runCatching {
                instrumentation.uiAutomation.grantRuntimePermission(
                    context.packageName,
                    Manifest.permission.RECORD_AUDIO,
                )
            }.onFailure { SpikeReport.note("grantRuntimePermission 失败：${it.message}") }
        }
        val recordGranted = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        val settingsGranted = context.checkSelfPermission(Manifest.permission.MODIFY_AUDIO_SETTINGS) ==
            PackageManager.PERMISSION_GRANTED
        SpikeReport.measure(CRITERION, "record_audio_granted", recordGranted)
        SpikeReport.measure(CRITERION, "modify_audio_settings_granted", settingsGranted)
        SpikeReport.measure(
            CRITERION,
            "permission_plugin",
            permissionPath(recordGranted, settingsDeclared && settingsGranted),
        )

        if (!recordGranted) {
            for (key in listOf("sample_rate_hz", "channel_count", "rms")) {
                SpikeReport.unavailable(CRITERION, key, "record_audio_not_granted")
            }
            return
        }
        capture()
    }

    private fun capture() {
        val minBuffer = AudioRecord.getMinBufferSize(RATE_HZ, CHANNEL, ENCODING)
        if (minBuffer <= 0) {
            for (key in listOf("sample_rate_hz", "channel_count", "rms")) {
                SpikeReport.unavailable(CRITERION, key, "audiorecord_rejects_16k_mono_pcm16")
            }
            return
        }
        val record = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            RATE_HZ,
            CHANNEL,
            ENCODING,
            maxOf(minBuffer, RATE_HZ * BYTES_PER_SAMPLE),
        )
        try {
            if (record.state != AudioRecord.STATE_INITIALIZED) {
                for (key in listOf("sample_rate_hz", "channel_count", "rms")) {
                    SpikeReport.unavailable(CRITERION, key, "audiorecord_state_${record.state}")
                }
                return
            }
            // 采集参数取自 AudioRecord 自己回报的值，而不是我们请求的值：请求 16 kHz 而
            // 设备给了 48 kHz 是真实存在的形态，抄回请求值会把它藏起来。
            SpikeReport.measure(CRITERION, "sample_rate_hz", record.sampleRate)
            SpikeReport.measure(CRITERION, "channel_count", record.channelCount)
            SpikeReport.measure(CRITERION, "audio_source", record.audioSource)
            SpikeReport.measure(CRITERION, "audio_format", record.audioFormat)

            record.startRecording()
            if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                SpikeReport.unavailable(CRITERION, "rms", "recording_state_${record.recordingState}")
                return
            }
            val stats = drain(record)
            SpikeReport.measure(CRITERION, "frames", stats.frames)
            SpikeReport.measure(CRITERION, "capture_ms", stats.elapsedMs)
            SpikeReport.measure(CRITERION, "peak_abs", stats.peak)
            if (stats.frames <= 0) {
                SpikeReport.unavailable(CRITERION, "rms", "no_frames_returned_by_audiorecord")
                return
            }
            SpikeReport.measure(CRITERION, "rms", stats.rms())
        } finally {
            runCatching { record.stop() }
            record.release()
        }
    }

    /**
     * 读满 [CAPTURE_MS]，并**丢掉开头 [WARMUP_MS]**。
     *
     * 丢弃不是为了让 RMS 好看：AudioRecord 在管道建立前会回填零样本，把它们算进 RMS
     * 只会稀释一个本来就极小的机架噪声值，让「采到静音」与「采到噪声」更难区分。
     */
    private fun drain(record: AudioRecord): CaptureStats {
        val buffer = ShortArray(RATE_HZ / 10)
        val started = System.nanoTime()
        var frames = 0L
        var energy = 0.0
        var peak = 0
        var warmupFrames = RATE_HZ * WARMUP_MS / 1000
        while ((System.nanoTime() - started) / 1_000_000 < CAPTURE_MS) {
            val read = record.read(buffer, 0, buffer.size)
            if (read <= 0) {
                continue
            }
            var index = 0
            if (warmupFrames > 0) {
                val skip = minOf(warmupFrames, read)
                warmupFrames -= skip
                index = skip
            }
            while (index < read) {
                val sample = buffer[index].toInt()
                energy += (sample.toDouble() * sample.toDouble())
                if (abs(sample) > peak) {
                    peak = abs(sample)
                }
                frames += 1
                index += 1
            }
        }
        return CaptureStats(frames, energy, peak, (System.nanoTime() - started) / 1_000_000)
    }

    private data class CaptureStats(
        val frames: Long,
        val energy: Double,
        val peak: Int,
        val elapsedMs: Long,
    ) {
        /** 归一化到 [0,1]：判据只问「是否大于零」，但归一化后的值在不同位深下可比。 */
        fun rms(): Double = sqrt(energy / frames.toDouble()) / FULL_SCALE
    }

    private fun declaredPermissions(context: android.content.Context): Set<String> {
        val info = context.packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_PERMISSIONS,
        )
        return info.requestedPermissions?.toSet() ?: emptySet()
    }

    /**
     * 与 `xtask` 的判据①阈值逐字对应的值。**失败时也要给出可读的状态**，
     * 而不是回一个空串——空串会让宿主侧判成未执行，掩盖「权限确实没拿到」。
     */
    private fun permissionPath(recordGranted: Boolean, settingsReady: Boolean): String {
        val record = if (recordGranted) "record_audio_granted" else "record_audio_denied"
        val settings = if (settingsReady) "modify_audio_settings_granted" else "modify_audio_settings_missing"
        return "$record+$settings"
    }

    private companion object {
        const val CRITERION = "microphone_capture"
        const val RATE_HZ = 16000
        const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        const val ENCODING = AudioFormat.ENCODING_PCM_16BIT
        const val BYTES_PER_SAMPLE = 2
        const val CAPTURE_MS = 1500L
        const val WARMUP_MS = 250
        const val FULL_SCALE = 32768.0
    }
}
