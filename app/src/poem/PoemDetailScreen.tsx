/**
 * 详情页。
 *
 * # 排布顺序是契约的一部分
 *
 * 原文 → 韵部 → 异说与归属 → 历代集评 → 分界 → AI 赏析。
 *
 * 前四块全是可复核的考据材料，共用 `sourced-block` 容器与 `data-provenance="sourced"`；
 * AI 面板在它们**全部之后**，容器与标记都不同。这不是审美排序：
 * 「绝不与原文或集评以可能被误认为考据成果的方式交错排布」这条要求，
 * 在 DOM 上的具体形态就是「顺序在后 + 容器不嵌套」，两者都有断言钉住。
 */

import { useEffect, useState } from "react";
import type { AppreciationState } from "../contracts/ai";
import type { PoemDetail } from "../contracts/core";
import type { AppreciationPort, PoemPort } from "../data/ports";
import AiAppreciationPanel from "./AiAppreciationPanel";
import AttributionPanel from "./AttributionPanel";
import CommentaryList from "./CommentaryList";
import OriginalText from "./OriginalText";
import RhymePanel from "./RhymePanel";
import "./poem.css";

export interface PoemDetailScreenProps {
  poemId: string;
  poemPort: PoemPort;
  appreciationPort: AppreciationPort;
  onBack: () => void;
}

export default function PoemDetailScreen({
  poemId,
  poemPort,
  appreciationPort,
  onBack,
}: PoemDetailScreenProps) {
  const [detail, setDetail] = useState<PoemDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showTones, setShowTones] = useState(false);
  const [appreciation, setAppreciation] = useState<AppreciationState>({ kind: "absent" });

  useEffect(() => {
    // `disposed` 守卫与标题栏那边同源：StrictMode 会二次调用 effect，
    // 卸载后写状态在开发期是一条警告、在生产期是一次泄漏。
    let disposed = false;
    setDetail(null);
    setError(null);
    poemPort
      .poemDetail({ poem_id: poemId })
      .then((result) => {
        if (!disposed) {
          setDetail(result);
        }
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setError(cause instanceof Error ? cause.message : "读取作品详情失败");
        }
      });
    return () => {
      disposed = true;
    };
  }, [poemId, poemPort]);

  useEffect(() => {
    let disposed = false;
    appreciationPort
      .appreciate({ poem_id: poemId })
      .then((state) => {
        if (!disposed) {
          setAppreciation(state);
        }
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setAppreciation({
            kind: "failed",
            message: cause instanceof Error ? cause.message : "AI 赏析获取失败",
          });
        }
      });
    return () => {
      disposed = true;
    };
  }, [poemId, appreciationPort]);

  if (error !== null) {
    return (
      <div className="poem-screen">
        <button type="button" className="poem-screen__back" onClick={onBack}>
          返回检索
        </button>
        <p className="poem-screen__error" role="alert" data-testid="detail-error">
          {error}
        </p>
      </div>
    );
  }

  if (detail === null) {
    return (
      <div className="poem-screen">
        <p className="poem-screen__loading" data-testid="detail-loading">
          正在读取…
        </p>
      </div>
    );
  }

  return (
    <div className="poem-screen" data-testid="poem-detail">
      <div className="poem-screen__bar">
        <button
          type="button"
          className="poem-screen__back"
          onClick={onBack}
          data-testid="detail-back">
          返回检索
        </button>
        <label className="poem-screen__toggle">
          <input
            type="checkbox"
            checked={showTones}
            onChange={(event) => {
              setShowTones(event.target.checked);
            }}
            data-testid="tone-toggle"
          />
          <span>标注平仄</span>
        </label>
      </div>

      <OriginalText poem={detail.poem} tones={detail.tones} showTones={showTones} />
      <RhymePanel groups={detail.rhyme_groups} />
      <AttributionPanel
        poem={detail.poem}
        provenance={detail.provenance}
        siblings={detail.work_group_siblings}
        conflicting={detail.attribution_conflict !== null}
      />
      <CommentaryList commentaries={detail.commentaries} />
      <AiAppreciationPanel state={appreciation} />
    </div>
  );
}
