package top.onethinker.yunjian

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp

/**
 * 全部界面。
 *
 * # 为什么用 `rememberSaveable` 存选中的 tab
 *
 * 「后台返回时页面不空白、视图不折叠」这条断言检的正是这件事。`remember` 在进程被回收
 * 后重建时会退回初值，于是从后台回来会落到第一个 tab——那在真机上表现为「我刚才在的
 * 那一页不见了」。`rememberSaveable` 让它经 `SavedStateHandle` 活过重建。
 *
 * # `imePadding` 与 `statusBarsPadding` 为什么都要
 *
 * 边到边窗口里 `adjustResize` 已被系统忽略（manifest 刻意不设 `windowSoftInputMode`），
 * 键盘遮挡必须由应用自己消费 ime 插入值。这两个修饰符是「键盘不遮挡输入框」的产品实现，
 * 不是排版偏好。
 */
@Composable
fun YunjianApp(viewModel: MainViewModel, modelDir: String) {
    val state by viewModel.state.collectAsState()
    var tab by rememberSaveable { mutableIntStateOf(0) }

    Surface(modifier = Modifier.fillMaxSize().testTag(TestTags.ROOT)) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .statusBarsPadding()
                .navigationBarsPadding()
                .imePadding()
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            CorpusBanner(state.corpus)

            state.error?.let { message ->
                Card(modifier = Modifier.fillMaxWidth().testTag(TestTags.ERROR_BANNER)) {
                    Text(text = message, modifier = Modifier.padding(8.dp))
                }
            }

            TabRow(selectedTabIndex = tab) {
                Tab(
                    selected = tab == 0,
                    onClick = { tab = 0 },
                    modifier = Modifier.testTag(TestTags.TAB_SEARCH),
                    text = { Text("检索") },
                )
                Tab(
                    selected = tab == 1,
                    onClick = { tab = 1 },
                    modifier = Modifier.testTag(TestTags.TAB_RECITE),
                    text = { Text("背诵") },
                )
                Tab(
                    selected = tab == 2,
                    onClick = { tab = 2 },
                    modifier = Modifier.testTag(TestTags.TAB_VOICE),
                    text = { Text("语音") },
                )
            }

            when (tab) {
                0 -> SearchAndReading(state, viewModel)
                1 -> RecitePane(state, viewModel)
                else -> VoicePane(state, viewModel, modelDir)
            }
        }
    }
}

@Composable
private fun CorpusBanner(corpus: CorpusState) {
    when (corpus) {
        CorpusState.Idle -> Text("尚未下载语料库", modifier = Modifier.testTag(TestTags.CORPUS_PROGRESS))
        is CorpusState.Working ->
            // 取用期间由进度块独占发言权：进度块与「尚未下载语料库」同屏共存会让界面上
            // 一句陈旧的话挨着「正在解压语料库」。桌面在 PR #108 修过同一处。
            Column(modifier = Modifier.fillMaxWidth().testTag(TestTags.CORPUS_PROGRESS)) {
                Text(
                    text = corpus.stage.detail,
                    modifier = Modifier.testTag(TestTags.CORPUS_PROGRESS_DETAIL),
                )
                val fraction = corpus.stage.fraction
                if (fraction != null) {
                    LinearProgressIndicator(
                        progress = { fraction },
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
            }
        is CorpusState.Ready ->
            Text(text = corpus.facts, modifier = Modifier.testTag(TestTags.CORPUS_FACTS))
        is CorpusState.Failed ->
            Text(
                text = "语料未就绪：${corpus.reason}",
                modifier = Modifier.testTag(TestTags.CORPUS_PROGRESS),
            )
    }
}

/**
 * 检索与阅读。
 *
 * # 阅读页独占一屏，不与结果列表挤同一片空间
 *
 * 两者同屏时阅读页一展开就把列表推出可视区，用户想点的「阅读」「背诵」按钮跑到屏幕外，
 * `performClick` 静默落空（真机第十九、二十轮实测）；把阅读页挪到列表尾部也不行，
 * 它自己变成屏幕外的那一个（第二十一轮实测，图证：列表完整可见、无正文卡片）。
 *
 * 靠滚动去凑是把布局问题推给调用方。让它独占一屏之后，「屏幕上现在是哪一页」只有两种
 * 答案，两边都不需要滚动才能被看到。返回按钮是真实需要的产品功能，不是为测试开的门。
 */
@Composable
private fun SearchAndReading(state: UiState, viewModel: MainViewModel) {
    state.reading?.let { reading ->
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = viewModel::closeReading,
                modifier = Modifier.testTag(TestTags.READING_BACK),
            ) { Text("返回检索") }
            ReadingView(reading)
        }
        return
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = state.query,
                onValueChange = viewModel::onQueryChange,
                label = { Text("正文或残句") },
                singleLine = true,
                modifier = Modifier.testTag(TestTags.SEARCH_FIELD),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(
                    onSearch = { viewModel.search() },
                ),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    imeAction = ImeAction.Search,
                ),
            )
            Button(onClick = viewModel::search, modifier = Modifier.testTag(TestTags.SEARCH_SUBMIT)) {
                Text("检索")
            }
        }

        if (state.searched) {
            Text(
                text = "命中 ${state.hits.size} 条",
                modifier = Modifier.testTag(TestTags.SEARCH_RESULT_COUNT),
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = state.directId,
                onValueChange = viewModel::onDirectIdChange,
                label = { Text("按标识直达") },
                singleLine = true,
                modifier = Modifier.testTag(TestTags.DIRECT_ID_FIELD),
            )
            Button(
                onClick = { viewModel.openReading(state.directId.trim()) },
                // 空标识时不发请求：内核会报「作品详情需要一个 stable_id」，那条错误
                // 横幅在真机截图里挂在语音页上，与用户当时在做的事毫无关系。
                // 拿一个已知无效的输入去问后端，然后把后端的抱怨转述给用户，是把
                // 输入校验推给了不该管这件事的一层。
                enabled = state.directId.isNotBlank(),
                modifier = Modifier.testTag(TestTags.DIRECT_ID_OPEN),
            ) { Text("打开") }
        }

        // **阅读页渲染在结果列表之后**。放在之前时它一展开就把列表挤出可视区，用户想点的
        // 「阅读」「背诵」按钮跑到屏幕外，而 `performScrollToNode` 只滚 LazyColumn 内部、
        // 滚不出被父级 Column 挤掉的部分（真机实测点击静默落空）。放在之后，列表位置不受
        // 阅读页影响，正文靠 LazyColumn 自己的滚动就能看到。
        LazyColumn(
            modifier = Modifier.fillMaxWidth().testTag(TestTags.SEARCH_RESULTS),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            items(state.hits) { hit ->
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("${TestTags.SEARCH_HIT_PREFIX}${hit.poemId}"),
                ) {
                    Column(modifier = Modifier.padding(8.dp)) {
                        Text(text = "${hit.title} · ${hit.author}")
                        Text(text = hit.snippet, style = MaterialTheme.typography.bodySmall)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(
                                onClick = { viewModel.openReading(hit.poemId) },
                                modifier = Modifier
                                    .testTag("${TestTags.SEARCH_HIT_READ_PREFIX}${hit.poemId}"),
                            ) { Text("阅读") }
                            Button(
                                onClick = { viewModel.startRecite(hit.poemId) },
                                modifier = Modifier
                                    .testTag("${TestTags.SEARCH_HIT_RECITE_PREFIX}${hit.poemId}"),
                            ) { Text("背诵") }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ReadingView(reading: PoemReading) {
    // tag 带 poem_id：让「屏幕上这一页是不是我要的那一页」可直接判定，见 TestTags 那条注释。
    Card(modifier = Modifier.fillMaxWidth().testTag("${TestTags.READING_POEM_PREFIX}${reading.poemId}")) {
        // 阅读页独占一屏，所以这里由它自己滚（长诗加集评会超过一屏）。
        // 曾经把本卡片放进 LazyColumn 的一个 item，那时**不能**有这一层——LazyColumn 给子项
        // 无限高约束，嵌套竖向滚动会以
        // `Vertically scrollable component was measured with an infinity maximum height` 崩掉。
        Column(
            modifier = Modifier
                .padding(8.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                text = "${reading.title} · ${reading.dynasty}${reading.author}",
                modifier = Modifier.testTag(TestTags.READING_TITLE),
            )
            Text(text = reading.body, modifier = Modifier.testTag(TestTags.READING_BODY))

            reading.commentaries.forEachIndexed { index, commentary ->
                Text(
                    text = commentary.text,
                    modifier = Modifier.testTag("${TestTags.READING_COMMENTARY_PREFIX}$index"),
                )
                // 出处与评语正文分开渲染：断言要求「带出处」，而把两者拼成一行时
                // 无法区分「有出处」与「正文里恰好提到了一部书」。
                Text(
                    text = "出处：${commentary.sourceTitle} ${commentary.sourceLocator}",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.testTag("${TestTags.READING_COMMENTARY_CITATION_PREFIX}$index"),
                )
            }

            reading.appreciation?.let { appreciation ->
                Text(
                    text = appreciation.text,
                    modifier = Modifier.testTag(TestTags.READING_APPRECIATION),
                )
                Text(
                    text = "本段由 ${appreciation.model} 生成，未经人工审校（来源：${appreciation.source}）",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.testTag(TestTags.READING_APPRECIATION_DISCLOSURE),
                )
            }
        }
    }
}

@Composable
private fun RecitePane(state: UiState, viewModel: MainViewModel) {
    // **必须可滚动。** 题目九行加 128 字默写框已经超过一屏，普通 `Column` 会把「提交」
    // 按钮与评分挤出布局——真机实测它们的 `boundsInWindow` 是 `0,0,0,0`（压根没被摆放），
    // 于是点击落空、评分永不出现。真人遇到的是同一件事：看得到默写框，却找不到提交按钮。
    Column(
        modifier = Modifier.verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        val session = state.reciteSession
        if (session == null) {
            Text(
                text = "先在检索页选一首作品并点「背诵」",
                modifier = Modifier.testTag(TestTags.RECITE_EMPTY),
            )
            return@Column
        }
        Text(text = session.prompt, modifier = Modifier.testTag(TestTags.RECITE_PROMPT))
        OutlinedTextField(
            value = state.reciteAnswer,
            onValueChange = viewModel::onReciteAnswerChange,
            label = { Text("默写") },
            modifier = Modifier.fillMaxWidth().testTag(TestTags.RECITE_ANSWER_FIELD),
        )
        Button(onClick = { viewModel.submitRecite() }, modifier = Modifier.testTag(TestTags.RECITE_SUBMIT)) {
            Text("提交")
        }
        state.reciteScore?.let { score ->
            Text(
                text = "完整度 ${score.completeness} 严格准确 ${score.accuracyStrict} " +
                    "正常 ${score.normalCount} 漏 ${score.deletionCount} " +
                    "增 ${score.insertionCount} 替 ${score.substitutionCount} " +
                    "拒绝=${score.isRejected}",
                modifier = Modifier.testTag(TestTags.RECITE_SCORE),
            )
        }
    }
}

@Composable
private fun VoicePane(state: UiState, viewModel: MainViewModel, modelDir: String) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // 参考文本取当前阅读的作品；没有时用第一条命中。两者都没有时按钮仍可按，
        // 由 ViewModel 报出具体原因，而不是让按钮变灰——变灰说不出原因。
        val poemId = state.reading?.poemId ?: state.hits.firstOrNull()?.poemId ?: ""
        val reference = state.reading?.body ?: state.hits.firstOrNull()?.snippet ?: ""
        Button(
            onClick = { viewModel.startVoiceRound(context, poemId, reference, modelDir) },
            modifier = Modifier.testTag(TestTags.VOICE_START),
        ) {
            Text("开始语音跟读")
        }

        when (val voice = state.voice) {
            VoiceState.Idle -> Text("未开始", modifier = Modifier.testTag(TestTags.VOICE_STATUS))
            is VoiceState.Listening ->
                Text(voice.detail, modifier = Modifier.testTag(TestTags.VOICE_STATUS))
            is VoiceState.Finished ->
                Text(voice.detail, modifier = Modifier.testTag(TestTags.VOICE_STATUS))
            is VoiceState.Degraded -> {
                Text(
                    text = voice.reason,
                    modifier = Modifier.testTag(TestTags.VOICE_DEGRADED_REASON),
                )
                Text(
                    text = "已降级到打字背诵",
                    modifier = Modifier.testTag(TestTags.VOICE_FALLBACK_TO_TYPING),
                )
            }
        }
    }
}
