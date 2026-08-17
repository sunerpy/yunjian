import SwiftUI

/// 全部界面。Android 是 `YunjianApp.kt` 的那组 Composable，这里是同结构的 SwiftUI 视图。
///
/// # 为什么用 `@SceneStorage` 存选中的页签
///
/// 「后台返回时页面不空白、视图不折叠」这条断言检的正是这件事。`@State` 在场景被系统回收
/// 后重建时会退回初值，于是从后台回来会落到第一个页签——真机上表现为「我刚才在的那一页
/// 不见了」。`@SceneStorage` 让它活过重建，与 Compose 的 `rememberSaveable` 一一对应。
///
/// # 键盘遮挡由系统 + `.scrollDismissesKeyboard` 处理
///
/// Android 上边到边窗口里 `adjustResize` 已被系统忽略，必须由应用自己消费 ime 插入值
/// （`imePadding()`）。iOS 的 `ScrollView` 默认会为键盘让位，所以对应的产品实现是
/// 「输入区放在可滚动容器里」而不是加一个 padding 修饰符——**同一个判据，不同的机制**。
/// 判据 `input_bottom_screen_px > 0` 两侧一样：输入框底边必须落在屏幕上一个正的坐标。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
struct ContentView: View {
    @ObservedObject var viewModel: MainViewModel
    let modelDir: String

    /// 0 检索 / 1 背诵 / 2 语音。与 Android 的 tab 序号相同。
    @SceneStorage("yunjian.tab") private var tab: Int = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CorpusBanner(corpus: viewModel.state.corpus)

            if let message = viewModel.state.error {
                Text(message)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.secondary.opacity(0.12))
                    .accessibilityIdentifier(TestTags.errorBanner)
            }

            HStack(spacing: 0) {
                tabButton(title: "检索", index: 0, identifier: TestTags.tabSearch)
                tabButton(title: "背诵", index: 1, identifier: TestTags.tabRecite)
                tabButton(title: "语音", index: 2, identifier: TestTags.tabVoice)
            }

            switch tab {
            case 0:
                SearchAndReading(viewModel: viewModel)
            case 1:
                RecitePane(viewModel: viewModel)
            default:
                VoicePane(viewModel: viewModel, modelDir: modelDir)
            }

            Spacer(minLength: 0)
        }
        .padding(12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityIdentifier(TestTags.root)
    }

    private func tabButton(title: String, index: Int, identifier: String) -> some View {
        Button(title) { tab = index }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(tab == index ? Color.accentColor.opacity(0.18) : Color.clear)
            .accessibilityIdentifier(identifier)
    }
}

/// 语料状态横幅。
///
/// 取用期间由进度块**独占发言权**：进度块与「尚未下载语料库」同屏共存会让界面上一句陈旧的话
/// 挨着「正在解压语料库」。Android 与桌面各修过同一处。
private struct CorpusBanner: View {
    let corpus: CorpusState

    var body: some View {
        switch corpus {
        case .idle:
            Text("尚未下载语料库").accessibilityIdentifier(TestTags.corpusProgress)
        case .working(let stage):
            VStack(alignment: .leading, spacing: 4) {
                Text(stage.detail).accessibilityIdentifier(TestTags.corpusProgressDetail)
                if let fraction = stage.fraction {
                    ProgressView(value: fraction)
                } else {
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityIdentifier(TestTags.corpusProgress)
        case .ready(let facts):
            Text(facts).accessibilityIdentifier(TestTags.corpusFacts)
        case .failed(let reason):
            Text("语料未就绪：\(reason)").accessibilityIdentifier(TestTags.corpusProgress)
        }
    }
}

/// 检索与阅读。
///
/// # 阅读页独占一屏，不与结果列表挤同一片空间
///
/// Android 真机上这条是逐层剥出来的：两者同屏时阅读页一展开就把列表推出可视区，用户想点的
/// 「阅读」「背诵」按钮跑到屏幕外，点击静默落空；把阅读页挪到列表尾部也不行，它自己变成
/// 屏幕外那一个。靠滚动去凑是把布局问题推给调用方。让它独占一屏之后，「屏幕上现在是哪一页」
/// 只有两种答案。**返回按钮是真实需要的产品功能，不是为测试开的门。**
private struct SearchAndReading: View {
    @ObservedObject var viewModel: MainViewModel

    var body: some View {
        if let reading = viewModel.state.reading {
            VStack(alignment: .leading, spacing: 8) {
                Button("返回检索") { viewModel.closeReading() }
                    .accessibilityIdentifier(TestTags.readingBack)
                ReadingView(reading: reading)
            }
        } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    TextField("正文或残句", text: Binding(
                        get: { viewModel.state.query },
                        set: viewModel.onQueryChange
                    ))
                    .textFieldStyle(.roundedBorder)
                    .submitLabel(.search)
                    .onSubmit { viewModel.search() }
                    .accessibilityIdentifier(TestTags.searchField)

                    Button("检索") { viewModel.search() }
                        .accessibilityIdentifier(TestTags.searchSubmit)
                }

                if viewModel.state.searched {
                    Text("命中 \(viewModel.state.hits.count) 条")
                        .accessibilityIdentifier(TestTags.searchResultCount)
                }

                HStack(spacing: 8) {
                    TextField("按标识直达", text: Binding(
                        get: { viewModel.state.directId },
                        set: viewModel.onDirectIdChange
                    ))
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier(TestTags.directIdField)

                    // 空标识时不发请求：内核会报「作品详情需要一个 stable_id」，而那条错误横幅
                    // 会挂在与用户当时所做之事毫无关系的页面上。拿一个已知无效的输入去问后端，
                    // 再把后端的抱怨转述给用户，是把输入校验推给了不该管这件事的一层。
                    Button("打开") { viewModel.openReading(poemId: viewModel.state.directId) }
                        .disabled(viewModel.state.directId.trimmingCharacters(in: .whitespaces).isEmpty)
                        .accessibilityIdentifier(TestTags.directIdOpen)
                }

                List(viewModel.state.hits) { hit in
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(hit.title) · \(hit.author)")
                        Text(hit.snippet).font(.footnote)
                        HStack(spacing: 8) {
                            Button("阅读") { viewModel.openReading(poemId: hit.poemId) }
                                .accessibilityIdentifier(TestTags.searchHitReadPrefix + hit.poemId)
                            Button("背诵") { viewModel.startRecite(poemId: hit.poemId) }
                                .accessibilityIdentifier(TestTags.searchHitRecitePrefix + hit.poemId)
                        }
                    }
                    .accessibilityIdentifier(TestTags.searchHitPrefix + hit.poemId)
                }
                .listStyle(.plain)
                .accessibilityIdentifier(TestTags.searchResults)
            }
        }
    }
}

/// 阅读页。
///
/// 标识里带 `poem_id`，让「屏幕上这一页是不是我要的那一页」可直接判定——Android 侧记过：
/// 只判「正文非空」会在上一首还开着时读到上一首，把一次装置问题记成产品 FAIL，比
/// NOT EXECUTED 更糟；而重开同一首时标题压根不变，判「标题变化」必然超时。**判身份。**
private struct ReadingView: View {
    let reading: PoemReading

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                Text("\(reading.title) · \(reading.dynasty)\(reading.author)")
                    .accessibilityIdentifier(TestTags.readingTitle)
                Text(reading.body)
                    .accessibilityIdentifier(TestTags.readingBody)

                ForEach(Array(reading.commentaries.enumerated()), id: \.offset) { index, commentary in
                    Text(commentary.text)
                        .accessibilityIdentifier(TestTags.readingCommentaryPrefix + String(index))
                    // 出处与评语正文**分开渲染**：断言要求「带出处」，而把两者拼成一行时无法
                    // 区分「有出处」与「正文里恰好提到了一部书」。
                    Text("出处：\(commentary.sourceTitle) \(commentary.sourceLocator)")
                        .font(.footnote)
                        .accessibilityIdentifier(TestTags.readingCommentaryCitationPrefix + String(index))
                }

                if let appreciation = reading.appreciation {
                    Text(appreciation.text)
                        .accessibilityIdentifier(TestTags.readingAppreciation)
                    Text("本段由 \(appreciation.model) 生成，未经人工审校（来源：\(appreciation.source)）")
                        .font(.footnote)
                        .accessibilityIdentifier(TestTags.readingAppreciationDisclosure)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
        }
        .accessibilityIdentifier(TestTags.readingPoemPrefix + reading.poemId)
    }
}

/// 背诵页。
///
/// **必须可滚动。** 题目九行加默写框已经超过一屏，固定布局会把「提交」按钮与评分挤出布局
/// ——Android 真机实测它们的 `boundsInWindow` 是 `0,0,0,0`（压根没被摆放），于是点击落空、
/// 评分永不出现。真人遇到的是同一件事：看得到默写框，却找不到提交按钮。
private struct RecitePane: View {
    @ObservedObject var viewModel: MainViewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                if let session = viewModel.state.reciteSession {
                    Text(session.prompt)
                        .accessibilityIdentifier(TestTags.recitePrompt)
                    TextField("默写", text: Binding(
                        get: { viewModel.state.reciteAnswer },
                        set: viewModel.onReciteAnswerChange
                    ), axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier(TestTags.reciteAnswerField)
                    Button("提交") { viewModel.submitRecite() }
                        .accessibilityIdentifier(TestTags.reciteSubmit)
                    if let score = viewModel.state.reciteScore {
                        // 文案与 Android 逐字相同：验收判据从这一行读
                        // completeness / accuracy_strict / 四类计数 / 是否拒绝。
                        Text(
                            "完整度 \(score.completeness) 严格准确 \(score.accuracyStrict) "
                                + "正常 \(score.normalCount) 漏 \(score.deletionCount) "
                                + "增 \(score.insertionCount) 替 \(score.substitutionCount) "
                                + "拒绝=\(score.isRejected)"
                        )
                        .accessibilityIdentifier(TestTags.reciteScore)
                    }
                } else {
                    Text("先在检索页选一首作品并点「背诵」")
                        .accessibilityIdentifier(TestTags.reciteEmpty)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollDismissesKeyboard(.interactively)
    }
}

/// 语音页。
private struct VoicePane: View {
    @ObservedObject var viewModel: MainViewModel
    let modelDir: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // 参考文本取当前阅读的作品；没有时用第一条命中。两者都没有时按钮仍可按，
            // 由 ViewModel 报出具体原因，而不是让按钮变灰——变灰说不出原因。
            let poemId = viewModel.state.reading?.poemId ?? viewModel.state.hits.first?.poemId ?? ""
            let reference = viewModel.state.reading?.body ?? viewModel.state.hits.first?.snippet ?? ""
            Button("开始语音跟读") {
                viewModel.startVoiceRound(poemId: poemId, reference: reference, modelDir: modelDir)
            }
            .accessibilityIdentifier(TestTags.voiceStart)

            switch viewModel.state.voice {
            case .idle:
                Text("未开始").accessibilityIdentifier(TestTags.voiceStatus)
            case .listening(let detail), .finished(let detail):
                Text(detail).accessibilityIdentifier(TestTags.voiceStatus)
            case .degraded(let reason):
                Text(reason).accessibilityIdentifier(TestTags.voiceDegradedReason)
                Text("已降级到打字背诵").accessibilityIdentifier(TestTags.voiceFallbackToTyping)
            }
        }
    }
}
