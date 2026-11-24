// 제품 내보내기: 대상(제품)마다 포맷 어댑터와 설정을 두고, 미리보기 후 파일로 내보낸다.
// 제품 키는 처음 내보낼 때 고정된다. 고정 전에는 규칙으로 제안하고, 직접 지정할 수도 있다.
import { useCallback, useEffect, useState } from "react";
import { ask, save } from "@tauri-apps/plugin-dialog";
import {
  api,
  type AdapterInfo,
  type ExportCheck,
  type ExportIssue,
  type ExportPreview,
  type ExportRow,
  type ExportTarget,
  type ExportTargetInput,
  type Locale,
  type TableConfig,
} from "../api";
import type { RunFn } from "../App";

const MAX_ROWS_SHOWN = 300;

/** 키 순서와 관계없이 같은 값이면 같은 문자열(서버는 JSON 키를 정렬해 돌려준다). */
function stableJson(v: unknown): string {
  return JSON.stringify(v, (_, x) =>
    x && typeof x === "object" && !Array.isArray(x)
      ? Object.fromEntries(Object.entries(x as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)))
      : x,
  );
}

export function defaultTableConfig(locales: Locale[]): TableConfig {
  return {
    sheet_name: "Strings",
    key_header: "Key",
    columns: locales.map((l) => ({ header: l.code, locale: l.code })),
    key_rule: { prefix: "", case: "keep", dot_replacement: null },
    fallback_to_source: false,
    include_deprecated: false,
  };
}

/** 빠진 필드는 서버 기본값과 같은 값으로 채운다(JSON 으로 만든 설정 등). */
function normalizeTableConfig(raw: unknown): TableConfig {
  const d = defaultTableConfig([]);
  const c = (raw ?? {}) as Partial<TableConfig>;
  return {
    ...d,
    ...c,
    columns: c.columns ?? [],
    key_rule: { ...d.key_rule, ...(c.key_rule ?? {}), dot_replacement: c.key_rule?.dot_replacement ?? null },
  };
}

export default function ExportView({ run }: { run: RunFn }) {
  const [locales, setLocales] = useState<Locale[]>([]);
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [targets, setTargets] = useState<ExportTarget[]>([]);
  const [draft, setDraft] = useState<ExportTargetInput | null>(null);
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [check, setCheck] = useState<ExportCheck | null>(null);
  const [reason, setReason] = useState("");
  const [message, setMessage] = useState<string | null>(null);

  const loadTargets = useCallback(async () => setTargets(await api.listExportTargets()), []);

  /** 미리보기와 검증 게이트(오류는 차단, 경고는 사유 필요)를 같이 읽는다. */
  const loadPreview = async (id: string | null) => {
    setPreview(id ? await api.planExport(id) : null);
    setCheck(id ? await api.checkExport(id) : null);
  };

  useEffect(() => {
    run(async () => {
      setLocales(await api.listLocales());
      setAdapters(await api.listAdapters());
      await loadTargets();
    });
  }, [run, loadTargets]);

  const select = (t: ExportTarget) => {
    setDraft({ ...t });
    setMessage(null);
    setReason("");
    run(() => loadPreview(t.id));
  };

  const saveDraft = () =>
    draft &&
    run(async () => {
      const id = await api.saveExportTarget(draft);
      const ts = await api.listExportTargets();
      setTargets(ts);
      const t = ts.find((x) => x.id === id);
      setDraft(t ? { ...t } : { ...draft, id });
      setMessage("저장함");
      await loadPreview(id);
    });

  const remove = () =>
    draft?.id &&
    run(async () => {
      if (!(await ask(`내보내기 대상 '${draft.name}'을 삭제할까요?`, { title: "대상 삭제", kind: "warning" }))) return;
      await api.deleteExportTarget(draft.id!);
      setDraft(null);
      await loadPreview(null);
      await loadTargets();
    });

  const exportFile = () =>
    draft?.id &&
    run(async () => {
      const path = await save({ filters: [{ name: "Excel", extensions: ["xlsx"] }], defaultPath: `${draft.name}.xlsx` });
      if (typeof path !== "string") return;
      const r = await api.exportToFile(draft.id!, path, reason.trim() || null);
      setMessage(`${r.rows}행을 내보냄. 새로 고정한 제품 키 ${r.new_keys}개${r.changeset_id ? ` (변경 묶음 #${r.changeset_id})` : ""}`);
      setReason("");
      await loadPreview(draft.id!);
    });

  const saved = targets.find((t) => t.id === draft?.id);
  const dirty = !!draft && !!saved && stableJson(saved) !== stableJson(draft);

  return (
    <div className="split">
      <div className="list">
        <div className="row" style={{ padding: "8px 12px", margin: 0 }}>
          <strong>내보내기 대상</strong>
          <button
            style={{ marginLeft: "auto" }}
            onClick={() => {
              setDraft({ id: null, name: "새 대상", adapter: "table", config: defaultTableConfig(locales) });
              setPreview(null);
              setMessage(null);
            }}
          >
            새 대상
          </button>
        </div>
        {targets.map((t) => (
          <div key={t.id} className={`list-item ${t.id === draft?.id ? "active" : ""}`} onClick={() => select(t)}>
            <div>{t.name}</div>
            <div className="muted">{t.adapter}</div>
          </div>
        ))}
        {targets.length === 0 && <div className="list-item muted">대상이 없습니다</div>}
      </div>

      <div className="detail">
        {!draft ? (
          <div className="muted">왼쪽에서 대상을 고르거나 새로 만드세요.</div>
        ) : (
          <>
            <div className="card">
              <div className="field">
                <span>이름</span>
                <input value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
                <span>포맷 어댑터</span>
                <select
                  value={draft.adapter}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      adapter: e.target.value,
                      config: e.target.value === "table" ? defaultTableConfig(locales) : {},
                    })
                  }
                >
                  {adapters.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.id} — {a.description}
                    </option>
                  ))}
                </select>
              </div>
              {draft.adapter === "table" ? (
                <TableConfigEditor
                  config={normalizeTableConfig(draft.config)}
                  locales={locales}
                  onChange={(config) => setDraft({ ...draft, config })}
                />
              ) : (
                <JsonConfigEditor value={draft.config} onChange={(config) => setDraft({ ...draft, config })} />
              )}
              <div className="row">
                <button className="primary" onClick={saveDraft}>
                  저장
                </button>
                {draft.id && (
                  <button className="danger" onClick={remove}>
                    삭제
                  </button>
                )}
                {dirty && draft.id && <span className="notice">저장하지 않은 변경이 있습니다(미리보기는 저장된 설정 기준)</span>}
                {message && <span>{message}</span>}
              </div>
            </div>

            {check && <GateCard check={check} reason={reason} onReason={setReason} />}
            {preview && draft.id && (
              <PreviewCard
                preview={preview}
                canExport={!!check && check.errors.length === 0 && (check.warnings.length === 0 || reason.trim() !== "")}
                onExport={exportFile}
                onFixKey={(row, key) =>
                  run(async () => {
                    await api.fixExternalKey(draft.id!, row.token_id, key);
                    await loadPreview(draft.id!);
                  })
                }
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}

function TableConfigEditor({
  config,
  locales,
  onChange,
}: {
  config: TableConfig;
  locales: Locale[];
  onChange: (c: TableConfig) => void;
}) {
  const set = <K extends keyof TableConfig>(k: K, v: TableConfig[K]) => onChange({ ...config, [k]: v });
  const rule = config.key_rule;
  return (
    <>
      <div className="field">
        <span>시트 이름</span>
        <input value={config.sheet_name} onChange={(e) => set("sheet_name", e.target.value)} />
        <span>키 열 머리글</span>
        <input value={config.key_header} onChange={(e) => set("key_header", e.target.value)} />
        <span>언어 열</span>
        <div>
          {config.columns.map((c, i) => (
            <div className="row" key={i}>
              <input
                placeholder="머리글"
                value={c.header}
                onChange={(e) =>
                  set(
                    "columns",
                    config.columns.map((x, j) => (j === i ? { ...x, header: e.target.value } : x)),
                  )
                }
              />
              <select
                value={c.locale}
                onChange={(e) =>
                  set(
                    "columns",
                    config.columns.map((x, j) => (j === i ? { ...x, locale: e.target.value } : x)),
                  )
                }
              >
                {locales.map((l) => (
                  <option key={l.code} value={l.code}>
                    {l.name} ({l.code})
                  </option>
                ))}
              </select>
              <button onClick={() => set("columns", config.columns.filter((_, j) => j !== i))}>빼기</button>
            </div>
          ))}
          <button
            disabled={locales.length === 0}
            onClick={() => set("columns", [...config.columns, { header: "", locale: locales[0]?.code ?? "" }])}
          >
            열 추가
          </button>
        </div>
        <span>제품 키 규칙</span>
        <div className="row">
          <input
            placeholder="접두어"
            value={rule.prefix}
            style={{ width: 100 }}
            onChange={(e) => set("key_rule", { ...rule, prefix: e.target.value })}
          />
          <select
            value={rule.case}
            onChange={(e) => set("key_rule", { ...rule, case: e.target.value as TableConfig["key_rule"]["case"] })}
          >
            <option value="keep">대소문자 그대로</option>
            <option value="upper">대문자로</option>
            <option value="lower">소문자로</option>
          </select>
          <label>
            <input
              type="checkbox"
              checked={rule.dot_replacement != null}
              onChange={(e) => set("key_rule", { ...rule, dot_replacement: e.target.checked ? "_" : null })}
            />
            '.'을 바꾸기
          </label>
          {rule.dot_replacement != null && (
            <input
              value={rule.dot_replacement}
              style={{ width: 40 }}
              onChange={(e) => set("key_rule", { ...rule, dot_replacement: e.target.value })}
            />
          )}
          <span className="muted">아직 키가 없는 토큰에만 씁니다. 한 번 내보낸 키는 바뀌지 않습니다.</span>
        </div>
        <span>옵션</span>
        <div className="row">
          <label>
            <input
              type="checkbox"
              checked={config.fallback_to_source}
              onChange={(e) => set("fallback_to_source", e.target.checked)}
            />
            번역이 없으면 원문으로 채우기
          </label>
          <label>
            <input
              type="checkbox"
              checked={config.include_deprecated}
              onChange={(e) => set("include_deprecated", e.target.checked)}
            />
            사용 중지(deprecated) 토큰도 내보내기
          </label>
        </div>
      </div>
    </>
  );
}

/** 표 어댑터가 아닌 Rust 전용 어댑터의 설정은 JSON 으로 고친다. */
function JsonConfigEditor({ value, onChange }: { value: unknown; onChange: (v: unknown) => void }) {
  const [text, setText] = useState(JSON.stringify(value, null, 2));
  const [bad, setBad] = useState(false);
  return (
    <div className="field">
      <span>설정(JSON)</span>
      <textarea
        rows={10}
        value={text}
        style={{ fontFamily: "monospace", borderColor: bad ? "#dc2626" : undefined }}
        onChange={(e) => {
          setText(e.target.value);
          try {
            onChange(JSON.parse(e.target.value));
            setBad(false);
          } catch {
            setBad(true);
          }
        }}
      />
    </div>
  );
}

/** 내보낼 셀의 검증 결과: 오류는 내보내기를 막고, 경고는 사유를 적어야 진행한다. */
function GateCard({ check, reason, onReason }: { check: ExportCheck; reason: string; onReason: (r: string) => void }) {
  if (check.errors.length === 0 && check.warnings.length === 0) {
    return <div className="card muted">검증: 내보낼 셀에 오류·경고가 없습니다</div>;
  }
  const list = (issues: ExportIssue[]) => (
    <ul style={{ margin: "4px 0 8px", paddingLeft: 18 }}>
      {issues.slice(0, 10).map((x, i) => (
        <li key={i}>
          {x.token_key} · {x.locale} · {x.validator}: {x.message}
        </li>
      ))}
      {issues.length > 10 && <li className="muted">… 외 {issues.length - 10}건(검증 탭에서 전체 보기)</li>}
    </ul>
  );
  return (
    <div className="card">
      {check.errors.length > 0 && (
        <>
          <div>
            <span className="badge error">오류 {check.errors.length}건</span> 고치기 전에는 내보낼 수 없습니다
          </div>
          {list(check.errors)}
        </>
      )}
      {check.warnings.length > 0 && (
        <>
          <div>
            <span className="badge warning">경고 {check.warnings.length}건</span> 사유를 적으면 내보낼 수 있습니다(변경 이력에 남음)
          </div>
          {list(check.warnings)}
          <input
            placeholder="경고를 두고 내보내는 사유"
            value={reason}
            onChange={(e) => onReason(e.target.value)}
            style={{ width: "100%" }}
          />
        </>
      )}
    </div>
  );
}

function PreviewCard({
  preview,
  canExport,
  onExport,
  onFixKey,
}: {
  preview: ExportPreview;
  canExport: boolean;
  onExport: () => void;
  onFixKey: (row: ExportRow, key: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const [editing, setEditing] = useState<{ tokenId: string; key: string } | null>(null);
  const q = filter.trim().toLowerCase();
  const rows = preview.rows.filter(
    (r) => !q || r.external_key.toLowerCase().includes(q) || r.token_key.toLowerCase().includes(q),
  );
  return (
    <div className="card">
      <div className="row">
        <strong>미리보기</strong>
        <span>
          {preview.rows.length}행 · 새로 고정될 제품 키 <b>{preview.new_keys}</b>개
        </span>
        <input placeholder="키·토큰 찾기" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="primary" style={{ marginLeft: "auto" }} disabled={!canExport} onClick={onExport}>
          파일로 내보내기
        </button>
      </div>
      <div className="table-scroll">
        <table className="table">
          <thead>
            <tr>
              <th>제품 키</th>
              <th>토큰</th>
              {preview.locales.map((l) => (
                <th key={l}>{l}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, MAX_ROWS_SHOWN).map((r) => (
              <tr key={r.token_id}>
                <td style={{ whiteSpace: "nowrap" }}>
                  {editing?.tokenId === r.token_id ? (
                    <form
                      className="row"
                      style={{ margin: 0 }}
                      onSubmit={(e) => {
                        e.preventDefault();
                        onFixKey(r, editing.key);
                        setEditing(null);
                      }}
                    >
                      <input
                        autoFocus
                        value={editing.key}
                        onChange={(e) => setEditing({ ...editing, key: e.target.value })}
                      />
                      <button type="submit">고정</button>
                      <button type="button" onClick={() => setEditing(null)}>
                        취소
                      </button>
                    </form>
                  ) : (
                    <>
                      {r.external_key}
                      {r.fixed ? (
                        <span className="badge gray">고정</span>
                      ) : (
                        <>
                          <span className="badge">제안</span>{" "}
                          <button onClick={() => setEditing({ tokenId: r.token_id, key: r.external_key })}>직접 지정</button>
                        </>
                      )}
                    </>
                  )}
                </td>
                <td>{r.token_key}</td>
                {preview.locales.map((l) => (
                  <td key={l} className="pre">
                    {r.values[l] ?? <span className="muted">(없음)</span>}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {rows.length > MAX_ROWS_SHOWN && <div className="muted">앞 {MAX_ROWS_SHOWN}행만 표시</div>}
    </div>
  );
}
