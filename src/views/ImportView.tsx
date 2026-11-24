// 엑셀 가져오기: 파일 → 열 매핑 → 미리보기(diff) → 적용(Import 변경 묶음 하나).
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  api,
  type ColumnMapping,
  type ImportOptions,
  type ImportPlan,
  type ImportResult,
  type ImportSource,
  type Locale,
  type SheetPreview,
} from "../api";
import type { RunFn } from "../App";
import MappingEditor, { mappingReady, suggestMapping } from "../components/MappingEditor";
import Change from "../components/Change";

export const SHEET_FILTERS = [{ name: "스프레드시트", extensions: ["xlsx", "xlsm", "xlsb", "xls", "ods"] }];
const MAX_ROWS_SHOWN = 500;

export default function ImportView({ run }: { run: RunFn }) {
  const [locales, setLocales] = useState<Locale[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [sheets, setSheets] = useState<SheetPreview[]>([]);
  const [mapping, setMapping] = useState<ColumnMapping | null>(null);
  const [options, setOptions] = useState<ImportOptions>({ create_missing_tokens: true, clear_empty: false });
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);

  useEffect(() => {
    run(async () => setLocales(await api.listLocales()));
  }, [run]);

  // 입력이 바뀌면 이전 미리보기는 버린다.
  useEffect(() => setPlan(null), [path, mapping, options]);

  const pickFile = () =>
    run(async () => {
      const p = await open({ filters: SHEET_FILTERS, multiple: false, directory: false });
      if (typeof p !== "string") return;
      const s = await api.workbookPreview(p);
      setPath(p);
      setSheets(s);
      setMapping(s[0] ? suggestMapping(s[0], 0, locales) : null);
      setResult(null);
    });

  const source: ImportSource | null = path && mappingReady(mapping) ? { kind: "mapped", path, mapping } : null;

  return (
    <div className="page">
      <h2>엑셀 가져오기</h2>
      {locales.length === 0 && <div className="notice">프로젝트에 언어가 없습니다. 문구 탭에서 언어를 먼저 추가하세요.</div>}

      <div className="card">
        <div className="row">
          <button onClick={pickFile}>파일 선택</button>
          <span className="path">{path ?? "xlsx, xlsm, xlsb, xls, ods"}</span>
        </div>
        {mapping && <MappingEditor sheets={sheets} locales={locales} mapping={mapping} onChange={setMapping} />}
      </div>

      {path && (
        <div className="card">
          <div className="row">
            <label>
              <input
                type="checkbox"
                checked={options.create_missing_tokens}
                onChange={(e) => setOptions({ ...options, create_missing_tokens: e.target.checked })}
              />
              없는 토큰은 새로 만들기
            </label>
            <label>
              <input
                type="checkbox"
                checked={options.clear_empty}
                onChange={(e) => setOptions({ ...options, clear_empty: e.target.checked })}
              />
              빈 칸은 값 지우기(끄면 빈 칸은 건너뜀)
            </label>
          </div>
          <div className="row">
            <button disabled={!source} onClick={() => source && run(async () => setPlan(await api.planImport(source, options)))}>
              미리보기
            </button>
            <button
              className="primary"
              disabled={!source || !plan || (plan.changes.length === 0 && plan.new_tokens.length === 0)}
              onClick={() =>
                source &&
                run(async () => {
                  setResult(await api.applyImport(source, options));
                  setPlan(null);
                })
              }
            >
              적용
            </button>
            {result && (
              <span>
                {result.changeset_id == null
                  ? "바뀐 내용이 없습니다"
                  : `변경 묶음 #${result.changeset_id}: 칸 ${result.changed_cells}개, 새 토큰 ${result.created_tokens}개`}
              </span>
            )}
          </div>
        </div>
      )}

      {plan && <PlanView plan={plan} />}
    </div>
  );
}

export function PlanView({ plan }: { plan: ImportPlan }) {
  const inserts = plan.changes.filter((c) => c.old == null).length;
  const deletes = plan.changes.filter((c) => c.new == null).length;
  const updates = plan.changes.length - inserts - deletes;
  const warnings: [string, string[]][] = [
    ["프로젝트에 없는 언어 열(건너뜀)", plan.unknown_locales],
    ["표 안에서 중복된 토큰명(뒤 행의 값을 씀)", plan.duplicate_keys],
    ["같은 토큰을 가리키는 다른 행이 있어 건너뜀", plan.conflicting_keys],
    ["프로젝트에 없는 토큰(건너뜀)", plan.skipped_tokens],
  ];
  return (
    <div className="card">
      <div className="stats">
        <span>
          토큰 행 <b>{plan.rows}</b>
        </span>
        <span>
          바뀌는 칸 <b>{plan.changes.length}</b> (새 값 {inserts} · 수정 {updates} · 지움 {deletes})
        </span>
        <span>
          새 토큰 <b>{plan.new_tokens.length}</b>
        </span>
        <span>
          같은 값 <b>{plan.unchanged}</b>
        </span>
      </div>
      {plan.renamed.length > 0 && (
        <div className="muted">
          이전 토큰명으로 찾은 행 {plan.renamed.length}개:{" "}
          {plan.renamed
            .slice(0, 20)
            .map((r) => `${r.sheet_key} → ${r.current_key}`)
            .join(", ")}
          {plan.renamed.length > 20 && " …"}
        </div>
      )}
      {warnings
        .filter(([, list]) => list.length > 0)
        .map(([label, list]) => (
          <div key={label} className="notice">
            {label} {list.length}개: {list.slice(0, 20).join(", ")}
            {list.length > 20 && " …"}
          </div>
        ))}
      {plan.changes.length > 0 && (
        <>
          <h4>
            바뀌는 칸{plan.changes.length > MAX_ROWS_SHOWN && ` (앞 ${MAX_ROWS_SHOWN}개만 표시)`}
          </h4>
          <div className="table-scroll">
            <table className="table">
              <thead>
                <tr>
                  <th>토큰</th>
                  <th>언어</th>
                  <th>변경</th>
                </tr>
              </thead>
              <tbody>
                {plan.changes.slice(0, MAX_ROWS_SHOWN).map((c, i) => (
                  <tr key={i}>
                    <td>
                      {c.token_key}
                      {c.token_id == null && <span className="badge">새 토큰</span>}
                    </td>
                    <td>{c.locale}</td>
                    <td>
                      <Change old={c.old} next={c.new} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}
