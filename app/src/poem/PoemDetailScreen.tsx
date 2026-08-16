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
import type { PoemAnnotation, PoemDetail } from "../contracts/core";
import type { AppreciationPort, PoemPort } from "../data/ports";
import { errorReason } from "../data/errorReason";
import {
  type AnnotationLayer,
  readAnnotationPreference,
  writeAnnotationPreference,
} from "./annotationPreferences";
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
  /**
   * 无提示主动回忆。为真时注音与平仄两层默认隐藏，无论持久化偏好是什么。
   *
   * 当前没有调用方把它设为真——背诵那条路径的接入不在本功能范围内。它先以可测的形式
   * 存在，是为了让「默认隐藏」与「揭示不判错」这两条现在就有断言钉住，而不是等接入时
   * 再重新论证一遍。
   */
  recall?: boolean;
  /** 用户主动揭示了某一层。**没有表示对错的入参**，揭示因此不可能自动判错。 */
  onReveal?: (layer: AnnotationLayer) => void;
}

export default function PoemDetailScreen({
  poemId,
  poemPort,
  appreciationPort,
  onBack,
  recall = false,
  onReveal,
}: PoemDetailScreenProps) {
  const [detail, setDetail] = useState<PoemDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [annotation, setAnnotation] = useState<PoemAnnotation | null>(null);
  // 两个开关各自从自己那个键初始化，写回也各写自己那个键。
  // 共用一个对象的话，「写一个顺手带上另一个」就成了一次普通手误，而它的表现是「我只开了
  // 拼音，平仄自己也亮了」。
  const [showTones, setShowTones] = useState(() => readAnnotationPreference("tones"));
  const [showPinyin, setShowPinyin] = useState(() => readAnnotationPreference("pinyin"));
  const [appreciation, setAppreciation] = useState<AppreciationState>({ kind: "absent" });

  const toggle = (layer: AnnotationLayer, enabled: boolean) => {
    writeAnnotationPreference(layer, enabled);
    (layer === "pinyin" ? setShowPinyin : setShowTones)(enabled);
  };

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
          setError(errorReason(cause, "读取作品详情失败"));
        }
      });
    return () => {
      disposed = true;
    };
  }, [poemId, poemPort]);

  useEffect(() => {
    // **整首一次批量预取，且不看开关状态。**
    // 依赖数组里刻意没有 `showPinyin`：把它放进来的话，每次切换开关都会重新取一次，
    // 而「切换开关不得触发查询」正是本功能的一条验收。取回的结果放在内存里，
    // 开关只决定显不显示。
    if (detail === null) {
      return;
    }
    let disposed = false;
    poemPort
      .poemAnnotations({ poem_id: poemId, body: detail.poem.body })
      .then((result) => {
        if (!disposed) {
          // 只认属于当前这一首的结果：换首时上一首的响应可能后到。
          setAnnotation(result.poem_id === poemId ? result : null);
        }
      })
      .catch(() => {
        // 注音取不到就不显示注音层，不编造读音，也不打断阅读。
        if (!disposed) {
          setAnnotation(null);
        }
      });
    return () => {
      disposed = true;
    };
  }, [poemId, poemPort, detail]);

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
            message: errorReason(cause, "AI 赏析获取失败"),
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
        <div className="poem-screen__toggles">
          <label className="poem-screen__toggle">
            <input
              type="checkbox"
              checked={showPinyin}
              onChange={(event) => {
                toggle("pinyin", event.target.checked);
              }}
              data-testid="pinyin-toggle"
            />
            <span>标注拼音</span>
          </label>
          <label className="poem-screen__toggle">
            <input
              type="checkbox"
              checked={showTones}
              onChange={(event) => {
                toggle("tones", event.target.checked);
              }}
              data-testid="tone-toggle"
            />
            <span>标注平仄</span>
          </label>
        </div>
      </div>

      <OriginalText
        poem={detail.poem}
        tones={detail.tones}
        showTones={showTones}
        annotation={annotation}
        showPinyin={showPinyin}
        {...(recall ? { recall } : {})}
        {...(onReveal ? { onReveal } : {})}
      />
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
