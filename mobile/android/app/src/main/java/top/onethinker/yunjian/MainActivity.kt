package top.onethinker.yunjian

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewmodel.CreationExtras
import java.io.File

/**
 * 唯一 Activity。
 *
 * # 为什么在这里而不是 `YunjianApplication` 触发首启物化
 *
 * 物化要下载 212 MiB 并解压数 GiB。放在 `Application.onCreate` 会让**每一次**进程启动
 * （包括 instrumentation 只为调一个门面方法而拉起的那次）都开始下载，而 `onCreate`
 * 必须尽快返回，否则系统按 ANR 处理。放在这里，界面能同时显示进度。
 *
 * `enableEdgeToEdge` 是刻意的：`targetSdk = 35` 下边到边已被系统强制，显式调用只是把
 * 这件事写明——真正处理插入值的是 `YunjianApp` 里的 `imePadding` / `statusBarsPadding`。
 */
class MainActivity : ComponentActivity() {
    /**
     * 本进程唯一那份 repository。
     *
     * `internal` 而不是 `private`：androidTest 与产品在同一个包下，验收断言要探测
     * 「本次 `.so` 是否含 native-voice」时必须复用它。自己再 new 一个会产生第二份
     * `NativeFacade`，两份同时持有同一个 SQLite 文件时写入方报 `database is locked`
     * （第十轮真机实测把整轮语料物化搞黄了）。**一个进程一份门面。**
     */
    internal lateinit var repository: YunjianRepository
        private set

    private companion object {
        /** 流式识别模型目录名。与 `models/cache/` 下的上游发布包同名。 */
        const val STREAMING_ASR_MODEL = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // **进程级单例，不是每次 `onCreate` 一个。**
        // 每次新建会产生第二份 `NativeFacade`，两份同时开同一个 SQLite 时写入方报
        // `database is locked`。Activity 重建、instrumentation 逐条测试各拉一次界面，
        // 都会走到这里，所以持有者必须是进程而不是 Activity。
        repository = YunjianRepository.shared(applicationContext)
        val viewModel = ViewModelProvider(
            this,
            object : ViewModelProvider.Factory {
                override fun <T : ViewModel> create(modelClass: Class<T>, extras: CreationExtras): T {
                    @Suppress("UNCHECKED_CAST")
                    return MainViewModel(repository) as T
                }
            },
        )[MainViewModel::class.java]

        // 只在真正缺语料时才开始下载。已在本地时也走同一条路径，由 `already_present`
        // 分支立刻报出来——跳过它会让「第二次启动界面一片空白」成为正常状态。
        //
        // **`materialize()` 自身必须幂等到「同一进程里只有一次真的在跑」。**
        // Activity 每次重建（旋屏、从后台被回收后返回、instrumentation 逐条测试各拉一次）
        // 都会走到这里；`ViewModel` 在配置变更时存活，但进程内多次 `onCreate` 仍会重复
        // 调用。两次物化并发时后一次报 `database is locked`——真机上十条里有六条被这条
        // 错误连带拖红，而报错文字完全不提「重复启动」。
        // 幂等由 `MainViewModel.materialize()` 自己用状态守住（见那边的 `Working` 判断）。
        viewModel.materialize()

        // `startAsr` 走的是**流式 transducer**（encoder / decoder / joiner + tokens），
        // 不是 whisper：whisper 是离线整段模型，拿不到边说边出的 partial。
        // 目录名与 `yunjian-voice::models` 的缓存布局一致。
        //
        // 权重优先取**外部私有目录**（`getExternalFilesDir`）：那里应用可读写、外部工具也
        // 能直接放文件，而权重是按需下载的大件（int8 189 MiB），放在这里便于替换与清理。
        // 找不到时退回内部 `filesDir`。
        //
        // **刻意不让外部工具往 `filesDir` 里塞东西**：真机实测 `run-as mkdir` 在
        // `files/yunjian/` 下建目录会破坏应用自己在同一棵树下的写权限
        // （探针 `app_can_create_siblings=false`），随后语料物化报
        // `database is locked` / `unable to open database file`，而那两条报错都不指向真因。
        val modelRoot = getExternalFilesDir(null) ?: filesDir
        val modelDir = File(File(modelRoot, "yunjian"), "models/$STREAMING_ASR_MODEL")
        setContent {
            YunjianApp(viewModel = viewModel, modelDir = modelDir.absolutePath)
        }
    }
}
