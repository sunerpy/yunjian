/**
 * 数据访问端口。
 *
 * # 为什么是接口而不是直接 `invoke`
 *
 * 本 todo（61）要在 IPC 层（todo 64）之前落地，而 todo 64 又被本 todo 阻塞——所以界面必须
 * 先把它需要的形状说清楚。这一组接口就是那份说明：todo 64 去实现它，
 * 测试用替身实现它，两侧对着同一份签名。
 *
 * 端口方法与 `yunjian-core` 的公开 API 一一对应，参数名也照抄，**没有一个是新发明的**：
 *
 * | 端口方法 | core API | 出处 |
 * | --- | --- | --- |
 * | `searchText` | `Yunjian::search_text(TextSearchRequest)` | `api.rs:134` |
 * | `browseByTag` | `Yunjian::browse_by_tag(TagBrowseRequest)` | `api.rs:236` |
 * | `listTags` | `Yunjian::list_tags()` | `api.rs:221` |
 * | `poemDetail` | `Yunjian::poem_detail(PoemDetailRequest)` | `api.rs:241` |
 *
 * # 主题筛选换的是查询入口，不是页内过滤
 *
 * 作者、朝代、体裁能在页内过滤，因为它们就在命中行上。**主题不在**——`MetaHit` 与
 * `TextSearchHit` 都没有 `tags` 字段，标签只在 `PoemDetail.tags` 与
 * `browse_by_tag` 的入参里。所以选定主题时走的是另一条 API，
 * 而不是把已取回的页再筛一遍。这一条不这么设计就只能编造一个不存在的字段。
 */

import type {
  MetaPage,
  PoemAnnotation,
  PoemDetail,
  SearchPage,
  TagSummary,
} from "../contracts/core";
import type { AppreciationState } from "../contracts/ai";

/** `TextSearchRequest`。`crates/yunjian-core/src/search/text.rs:9-18`。 */
export interface TextSearchRequest {
  query: string;
  /** 服务端硬上限 100（`TEXT_SEARCH_HARD_CAP`，`text.rs:6-7`）。 */
  limit: number;
  cursor: string | null;
}

/** `TagBrowseRequest`，由 `api.rs:25-36` 的 `paged_request!` 宏展开。 */
export interface TagBrowseRequest {
  tag: string;
  cursor: string | null;
}

/** `PoemDetailRequest`。`crates/yunjian-core/src/api.rs:117-122`。 */
export interface PoemDetailRequest {
  poem_id: string;
}

/** 检索端口。 */
export interface SearchPort {
  searchText(request: TextSearchRequest): Promise<SearchPage>;
  browseByTag(request: TagBrowseRequest): Promise<MetaPage>;
  listTags(): Promise<TagSummary[]>;
}

/**
 * 整首注音的请求。
 *
 * 正文由调用方带下来。详情页已经拿着正文了，让后端再查一次会把一次批量预取变成两次
 * 查询；而注音本身是纯解析，不需要语料库。
 */
export interface PoemAnnotationRequest {
  poem_id: string;
  body: string;
}

/**
 * 阅读端口。
 *
 * `poemAnnotations` **刻意与 `poemDetail` 同属一个端口**：注音是「读这一首」的一部分，
 * 而把它放进同一个接口意味着每一处构造端口的地方都必须显式提供它，漏了是编译错误而不是
 * 一个运行期才发现的空注音层。
 */
export interface PoemPort {
  poemDetail(request: PoemDetailRequest): Promise<PoemDetail>;
  poemAnnotations(request: PoemAnnotationRequest): Promise<PoemAnnotation>;
}

/**
 * AI 赏析端口。
 *
 * 返回 `AppreciationState` 而不是裸 `Appreciation`：「没有配置密钥」与「生成失败」都是
 * 面板必须如实显示的状态，而不是异常。把它们变成 `throw` 会让调用方用一个 catch
 * 把两种完全不同的处境合并成一句「出错了」。
 */
export interface AppreciationPort {
  appreciate(request: PoemDetailRequest): Promise<AppreciationState>;
}
