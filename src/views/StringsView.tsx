// 문구 표: 행 = 토큰, 열 = 언어. Glide Data Grid(캔버스, 가상 스크롤)로 그린다.
// - 셀 편집·여러 셀 붙여넣기·Delete 로 지우기는 모두 onCellsEdited 한 곳으로 모여 변경 묶음 하나가 된다.
// - 복사는 getCellsForSelection 으로 Glide 가 탭 구분 텍스트를 만든다(엑셀에 바로 붙여넣기 가능).
// - 표를 다시 읽을 때마다 전체 검증도 다시 돌려 위반 셀에 색을 칠한다(캐시 없음).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CompactSelection,
  DataEditor,
  GridCellKind,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  type GridColumn,
  type GridSelection,
  type Item,
  type Theme,
} from "@glideapps/glide-data-grid";
import "@glideapps/glide-data-grid/dist/index.css";
import {
  api,
  type CellEdit,
  type CellFindings,
  type Grid,
  type GridRow,
  type RevRow,
  type Severity,
  type ValidationReport,
} from "../api";
import type { RunFn } from "../App";
import { SeverityBadge, metricsText } from "./ValidationView";

type Filters = {
  text: string;
  /** "" = 전체 언어 */
  locale: string;
  missing: boolean;
  outdated: boolean;
  failing: boolean;
};

const NO_FILTERS: Filters = { text: "", locale: "", missing: false, outdated: false, failing: false };

type Selected = { tokenId: string; locale: string | null };
export type CellRef = { tokenId: string; locale: string };

const KEY_COL_WIDTH = 240;
const LOCALE_COL_WIDTH = 260;
export const CELL_BG: Record<"missing" | "outdated" | Severity, string> = {
  missing: "#f1f5f9",
  outdated: "#fef3c7",
  error: "#fee2e2",
  warning: "#ffedd5",
  info: "#e0f2fe",
};
const SEVERITY_RANK: Record<Severity, number> = { error: 3, warning: 2, info: 1 };

const THEME: Partial<Theme> = {
  fontFamily: '"Pretendard", "Malgun Gothic", system-ui, sans-serif',
  baseFontStyle: "13px",
  headerFontStyle: "600 13px",
  editorFontSize: "13px",
};

const EMPTY_SELECTION: GridSelection = { columns: CompactSelection.empty(), rows: CompactSelection.empty() };

const cellId = (tokenId: string, locale: string) => `${tokenId}\u0000${locale}`;

/** 셀별 검증 결과와 가장 심한 심각도. */
function indexReport(report: ValidationReport | null) {
  const byCell = new Map<string, CellFindings>();
  const worst = new Map<string, Severity>();
  for (const c of report?.cells ?? []) {
    const id = cellId(c.token_id, c.locale);
    byCell.set(id, c);
    const top = c.findings.reduce<Severity | null>(
      (w, f) => (w == null || SEVERITY_RANK[f.severity] > SEVERITY_RANK[w] ? f.severity : w),
      null,
    );
    if (top) worst.set(id, top);
  }
  return { byCell, worst };
}

/** 검색어는 토큰명과 모든 언어 값에서 찾는다. 미번역·원문 변경·검증 실패는 선택한 언어(없으면 전체)에서 본다. */
export function filterRows(grid: Grid, f: Filters, worst: Map<string, Severity>): GridRow[] {
  const q = f.text.trim().toLowerCase();
  const locales = f.locale ? [f.locale] : grid.locales.map((l) => l.code);
  return grid.rows.filter((r) => {
    if (
      q &&
      !r.token.key.toLowerCase().includes(q) &&
      !Object.values(r.cells).some((c) => c.value.toLowerCase().includes(q))
    ) {
      return false;
    }
    if (f.missing && !locales.some((l) => !r.cells[l])) return false;
    if (f.outdated && !locales.some((l) => r.cells[l]?.outdated)) return false;
    if (f.failing && !locales.some((l) => worst.has(cellId(r.token.id, l)))) return false;
    return true;
  });
}

export default function StringsView({
  run,
  focus,
  onFocusDone,
}: {
  run: RunFn;
  /** 다른 화면(검증 결과 등)에서 열어 달라고 한 셀 */
  focus: CellRef | null;
  onFocusDone: () => void;
}) {
  const [grid, setGrid] = useState<Grid | null>(null);
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [selection, setSelection] = useState<GridSelection>(EMPTY_SELECTION);
  const [selected, setSelected] = useState<Selected | null>(null);
  const [history, setHistory] = useState<RevRow[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const editorRef = useRef<DataEditorRef>(null);

  const reload = useCallback(
    () =>
      run(async () => {
        setGrid(await api.loadGrid());
        setReport(await api.validateAll());
      }),
    [run],
  );
  useEffect(() => {
    reload();
  }, [reload]);

  const { byCell, worst } = useMemo(() => indexReport(report), [report]);
  const rows = useMemo(() => (grid ? filterRows(grid, filters, worst) : []), [grid, filters, worst]);

  // 다른 화면에서 요청한 셀로 이동: 필터를 풀고 그 셀을 고른 뒤 보이게 스크롤
  useEffect(() => {
    if (!focus || !grid) return;
    const row = grid.rows.findIndex((r) => r.token.id === focus.tokenId);
    const col = grid.locales.findIndex((l) => l.code === focus.locale) + 1;
    onFocusDone();
    if (row < 0 || col < 1) return;
    setFilters(NO_FILTERS);
    setSelection({ ...EMPTY_SELECTION, current: { cell: [col, row], range: { x: col, y: row, width: 1, height: 1 }, rangeStack: [] } });
    setSelected({ tokenId: focus.tokenId, locale: focus.locale });
    requestAnimationFrame(() => editorRef.current?.scrollTo(col, row, "both", 0, 0, { vAlign: "center" }));
  }, [focus, grid, onFocusDone]);

  const columns: GridColumn[] = useMemo(
    () =>
      grid
        ? [
            { id: "key", title: "토큰", width: widths.key ?? KEY_COL_WIDTH },
            ...grid.locales.map((l) => ({
              id: l.code,
              title: `${l.name} · ${l.code}${l.is_source ? " (원문)" : ""}`,
              width: widths[l.code] ?? LOCALE_COL_WIDTH,
            })),
          ]
        : [],
    [grid, widths],
  );

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const r = rows[row];
      if (!r || !grid) {
        return { kind: GridCellKind.Text, data: "", displayData: "", allowOverlay: false };
      }
      if (col === 0) {
        return { kind: GridCellKind.Text, data: r.token.key, displayData: r.token.key, allowOverlay: true };
      }
      const locale = grid.locales[col - 1].code;
      const cell = r.cells[locale];
      const sev = worst.get(cellId(r.token.id, locale));
      const bg = sev ? CELL_BG[sev] : !cell ? CELL_BG.missing : cell.outdated ? CELL_BG.outdated : undefined;
      return {
        kind: GridCellKind.Text,
        data: cell?.value ?? "",
        displayData: cell?.value ?? "",
        allowOverlay: true,
        themeOverride: bg ? { bgCell: bg } : undefined,
      };
    },
    [rows, grid, worst],
  );

  // 편집·붙여넣기·지우기. 토큰명 열은 한 칸 편집일 때만 이름 변경으로 받는다(여러 칸 붙여넣기로 이름이 바뀌지 않게).
  const onCellsEdited = useCallback(
    (edits: readonly { location: Item; value: EditableGridCell }[]) => {
      if (!grid) return true;
      const cellEdits: CellEdit[] = [];
      let rename: { tokenId: string; key: string } | null = null;
      let skippedKeys = 0;
      for (const { location, value } of edits) {
        const [col, row] = location;
        const r = rows[row];
        if (!r || value.kind !== GridCellKind.Text) continue;
        if (col === 0) {
          if (edits.length === 1) rename = { tokenId: r.token.id, key: value.data };
          else skippedKeys++;
          continue;
        }
        const locale = grid.locales[col - 1]?.code;
        if (locale) cellEdits.push({ token_id: r.token.id, locale, value: value.data });
      }
      setNotice(skippedKeys > 0 ? `토큰명 열 ${skippedKeys}칸은 붙여넣지 않음(이름 변경은 한 칸씩)` : null);
      run(async () => {
        if (rename) await api.renameToken(rename.tokenId, rename.key);
        if (cellEdits.length > 0) await api.setCells(cellEdits);
        await reload();
      });
      return true;
    },
    [grid, rows, run, reload],
  );

  const onSelectionChange = (s: GridSelection) => {
    setSelection(s);
    const c = s.current?.cell;
    const r = c ? rows[c[1]] : undefined;
    if (!c || !r || !grid) return setSelected(null);
    setSelected({ tokenId: r.token.id, locale: c[0] > 0 ? (grid.locales[c[0] - 1]?.code ?? null) : null });
  };

  useEffect(() => {
    if (!selected?.locale) return setHistory([]);
    const { tokenId, locale } = selected;
    run(async () => setHistory(await api.cellHistory(tokenId, locale)));
  }, [selected, grid, run]);

  if (!grid) return null;
  const selRow = selected ? grid.rows.find((r) => r.token.id === selected.tokenId) : undefined;
  const selCell = selRow && selected?.locale ? selRow.cells[selected.locale] : undefined;
  const selFindings = selRow && selected?.locale ? byCell.get(cellId(selRow.token.id, selected.locale)) : undefined;
  const check = (k: keyof Filters, label: string) => (
    <label>
      <input type="checkbox" checked={filters[k] as boolean} onChange={(e) => setFilters({ ...filters, [k]: e.target.checked })} />
      {label}
    </label>
  );

  return (
    <div className="strings">
      <AddForms run={run} reload={reload} hasLocales={grid.locales.length > 0} />

      <div className="filters">
        <input
          className="search"
          placeholder="검색(토큰명, 문구)"
          value={filters.text}
          onChange={(e) => setFilters({ ...filters, text: e.target.value })}
        />
        <select value={filters.locale} onChange={(e) => setFilters({ ...filters, locale: e.target.value })}>
          <option value="">전체 언어</option>
          {grid.locales.map((l) => (
            <option key={l.code} value={l.code}>
              {l.name} ({l.code})
            </option>
          ))}
        </select>
        {check("missing", "미번역")}
        {check("outdated", "원문 변경됨")}
        {check("failing", "검증 실패")}
        <span className="muted">
          {rows.length} / {grid.rows.length}개 토큰
        </span>
        {notice && <span className="notice">{notice}</span>}
        <span className="legend">
          <i style={{ background: CELL_BG.missing }} /> 미번역 <i style={{ background: CELL_BG.outdated }} /> 원문 변경됨
          <i style={{ background: CELL_BG.error }} /> 오류 <i style={{ background: CELL_BG.warning }} /> 경고
          <i style={{ background: CELL_BG.info }} /> 정보
        </span>
      </div>

      <div className="body">
        {grid.locales.length === 0 ? (
          <div className="empty">언어를 먼저 추가하세요</div>
        ) : (
          <div className="grid-wrap">
            <DataEditor
              ref={editorRef}
              columns={columns}
              rows={rows.length}
              getCellContent={getCellContent}
              onCellsEdited={onCellsEdited}
              getCellsForSelection={true}
              onPaste={true}
              gridSelection={selection}
              onGridSelectionChange={onSelectionChange}
              onColumnResize={(col, size) => setWidths((w) => ({ ...w, [col.id ?? col.title]: size }))}
              freezeColumns={1}
              rowMarkers="number"
              rowHeight={32}
              smoothScrollX
              smoothScrollY
              theme={THEME}
              width="100%"
              height="100%"
            />
          </div>
        )}

        <aside className="side">
          {!selRow ? (
            <div className="muted">셀을 선택하세요</div>
          ) : (
            <>
              <h3>{selRow.token.key}</h3>
              <TagEditor
                key={selRow.token.id}
                tags={selRow.token.tags}
                onSave={(tags) =>
                  run(async () => {
                    await api.setTokenTags(selRow.token.id, tags);
                    await reload();
                  })
                }
              />
              {selected?.locale ? (
                <>
                  <div className="muted">
                    {selected.locale} · {selCell ? selCell.status : "미번역"}
                    {selCell?.outdated && " · 원문 변경됨"}
                  </div>
                  {selFindings && (
                    <>
                      <h4>검증</h4>
                      {selFindings.findings.map((f, i) => (
                        <div key={i} style={{ marginBottom: 6 }}>
                          <SeverityBadge s={f.severity} /> {f.validator}
                          <div className="pre">
                            {f.error ? (
                              <span className="notice">검증기 오류: {f.error}</span>
                            ) : (
                              <>
                                {f.outcome?.message} <span className="muted">{metricsText(f.outcome?.metrics)}</span>
                              </>
                            )}
                          </div>
                        </div>
                      ))}
                    </>
                  )}
                  <h4>이력</h4>
                  <ol className="history">
                    {history.map((h) => (
                      <li key={h.changeset_id}>
                        <div className="muted">
                          #{h.changeset_id} {h.op} · {h.author} · {h.created_at}
                        </div>
                        <div className="pre">
                          <del>{h.old?.value ?? "∅"}</del> → <ins>{h.new?.value ?? "∅"}</ins>
                        </div>
                      </li>
                    ))}
                  </ol>
                </>
              ) : (
                <div className="muted">언어 열의 셀을 고르면 검증 결과와 이력이 보입니다</div>
              )}
            </>
          )}
        </aside>
      </div>
    </div>
  );
}

/** 토큰 태그(쉼표로 구분). 검증 규칙의 "태그" 범위에 쓴다. */
function TagEditor({ tags, onSave }: { tags: string[]; onSave: (tags: string[]) => void }) {
  const [text, setText] = useState(tags.join(", "));
  const parsed = text
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);
  const norm = (xs: string[]) => [...new Set(xs)].sort().join("\u0000");
  const changed = norm(parsed) !== norm(tags);
  return (
    <form
      className="row"
      onSubmit={(e) => {
        e.preventDefault();
        onSave(parsed);
      }}
    >
      <input placeholder="태그(쉼표로 구분)" value={text} onChange={(e) => setText(e.target.value)} style={{ flex: 1 }} />
      <button type="submit" disabled={!changed}>
        태그 저장
      </button>
    </form>
  );
}

function AddForms({ run, reload, hasLocales }: { run: RunFn; reload: () => Promise<void>; hasLocales: boolean }) {
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [isSource, setIsSource] = useState(!hasLocales);
  const [key, setKey] = useState("");

  useEffect(() => setIsSource(!hasLocales), [hasLocales]);

  return (
    <div className="forms">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          run(async () => {
            await api.addLocale(code, name || code, isSource);
            setCode("");
            setName("");
            await reload();
          });
        }}
      >
        <input placeholder="언어 코드 (ko-KR)" value={code} onChange={(e) => setCode(e.target.value)} required />
        <input placeholder="이름 (한국어)" value={name} onChange={(e) => setName(e.target.value)} />
        <label>
          <input type="checkbox" checked={isSource} onChange={(e) => setIsSource(e.target.checked)} /> 원문
        </label>
        <button type="submit">언어 추가</button>
      </form>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          run(async () => {
            await api.createToken(key);
            setKey("");
            await reload();
          });
        }}
      >
        <input placeholder="토큰명 (home.title)" value={key} onChange={(e) => setKey(e.target.value)} required />
        <button type="submit">토큰 추가</button>
      </form>
    </div>
  );
}
