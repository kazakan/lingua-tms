// 엔진이 잰 결과: 폭·줄 수·높이와 줄마다 끊긴 문구. 막대 길이는 엔진이 준 폭이다(화면 글꼴로 다시 그리지 않음).
import type { LayoutView } from "../api";

const px = (n: number) => `${Math.round(n * 10) / 10}px`;

export default function MeasureResult({ layout }: { layout: LayoutView }) {
  const limit = layout.limit ?? 0;
  const max = Math.max(limit, ...layout.lines.map((l) => l.width), 1);
  const over = limit > 0 && layout.width > limit;
  return (
    <div className="measure">
      <div className="muted">
        폭 <b className={over ? "over-text" : ""}>{px(layout.width)}</b>
        {limit > 0 && ` / ${layout.box_width > 0 ? "박스" : "한도"} ${px(limit)}`} · {layout.lines.length}줄 · 높이{" "}
        {px(layout.height)}
      </div>
      {layout.missing_glyphs > 0 && <div className="notice">폰트에 없는 글자 {layout.missing_glyphs}개</div>}
      {layout.word_overflow && <div className="notice">박스보다 넓은 단어가 있음</div>}
      <div className="lines">
        {layout.lines.map((l, i) => (
          <div key={i} className="line">
            <div className="line-bar">
              <span className={limit > 0 && l.width > limit ? "over" : ""} style={{ width: `${(l.width / max) * 100}%` }} />
              {limit > 0 && <i style={{ left: `calc(${(limit / max) * 100}% - 1px)` }} title={`한도 ${px(limit)}`} />}
            </div>
            <div className="line-text">
              <span className="pre">{l.text === "" ? " " : l.text}</span> <span className="muted">{px(l.width)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
