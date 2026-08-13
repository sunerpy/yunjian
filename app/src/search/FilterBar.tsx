/**
 * 筛选栏：作者、朝代、体裁、主题，以及「不折叠异说」开关。
 *
 * # 两条如实说明必须显示在界面上
 *
 * 1. **作者/朝代/体裁是页内过滤。** `TextSearchRequest` 只有 query/limit/cursor
 *    （`crates/yunjian-core/src/search/text.rs:9-18`），没有服务端过滤入口；CLI 的同名参数
 *    文档也写着「在本页内过滤」（`crates/yunjian-cli/src/cli.rs:63`）。不说这件事，
 *    用户会把「本页没有杜甫」读成「杜甫没写过」。
 * 2. **主题换的是查询入口。** 标签不在命中行上，只能走 `browse_by_tag`。所以选了主题就等于
 *    换了一次检索，检索词会被忽略——这一点也要写出来。
 */

import type { TagSummary } from "../contracts/core";
import type { RowFilters } from "../data/rows";

export interface FilterBarProps {
  filters: RowFilters;
  onFiltersChange: (next: RowFilters) => void;
  tags: TagSummary[];
  theme: string;
  onThemeChange: (next: string) => void;
  dedup: boolean;
  onDedupChange: (next: boolean) => void;
  /** 页内过滤是否有可能误导——即当前结果集来自分页检索而非全量。 */
  showPageScopeNote: boolean;
}

export default function FilterBar({
  filters,
  onFiltersChange,
  tags,
  theme,
  onThemeChange,
  dedup,
  onDedupChange,
  showPageScopeNote,
}: FilterBarProps) {
  return (
    <section className="filter-bar" aria-label="筛选">
      <div className="filter-bar__row">
        <label className="filter-bar__field">
          <span className="filter-bar__label">作者</span>
          <input
            type="text"
            className="filter-bar__input"
            value={filters.author}
            onChange={(event) => {
              onFiltersChange({ ...filters, author: event.target.value });
            }}
          />
        </label>

        <label className="filter-bar__field">
          <span className="filter-bar__label">朝代</span>
          <input
            type="text"
            className="filter-bar__input"
            value={filters.dynasty}
            onChange={(event) => {
              onFiltersChange({ ...filters, dynasty: event.target.value });
            }}
          />
        </label>

        <label className="filter-bar__field">
          <span className="filter-bar__label">体裁</span>
          <input
            type="text"
            className="filter-bar__input"
            value={filters.genre}
            onChange={(event) => {
              onFiltersChange({ ...filters, genre: event.target.value });
            }}
          />
        </label>

        <label className="filter-bar__field">
          <span className="filter-bar__label">主题</span>
          <select
            className="filter-bar__input"
            value={theme}
            onChange={(event) => {
              onThemeChange(event.target.value);
            }}>
            <option value="">不限</option>
            {tags.map((tag) => (
              <option key={tag.name} value={tag.name}>
                {tag.name}（{tag.poem_count}）
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="filter-bar__row">
        <label className="filter-bar__toggle">
          <input
            type="checkbox"
            checked={!dedup}
            onChange={(event) => {
              onDedupChange(!event.target.checked);
            }}
            data-testid="no-dedup-toggle"
          />
          <span>显示全部异说，不折叠</span>
        </label>
      </div>

      {showPageScopeNote && (
        <p className="filter-bar__note" data-testid="page-scope-note">
          作者、朝代与体裁只在当前这一页内过滤：后端的正文检索没有服务端过滤入口，
          所以「本页没有」不等于「语料里没有」。
        </p>
      )}
      {theme !== "" && (
        <p className="filter-bar__note" data-testid="theme-scope-note">
          按主题浏览走的是另一条查询，检索词此时不参与。
        </p>
      )}
    </section>
  );
}
