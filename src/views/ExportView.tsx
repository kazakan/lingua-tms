// 제품 내보내기: 대상(제품)마다 포맷 어댑터와 설정을 두고, 미리보기 후 파일로 내보낸다.
// 제품 키는 처음 내보낼 때 고정된다. 고정 전에는 규칙으로 제안하고, 직접 지정할 수도 있다.
import { useCallback, useEffect, useState } from "react";
import { ask, save } from "@tauri-apps/plugin-dialog";
import {
  api,
  type AdapterInfo,
  type CommandConfig,
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

export function defaultCommandConfig(locales: Locale[]): CommandConfig {
  return {
    export_command: "",
    import_command: "",
    locales: locales.map((l) => l.code),
    file_extension: "xlsx",
    timeout_secs: 60,
    options: {},
    key_rule: { prefix: "", case: "keep", dot_replacement: null },
    fallback_to_source: false,
    include_deprecated: false,
  };
}

/** 빠진 필드는 서버 기본값과 같은 값으로 채운다. */
export function normalizeCommandConfig(raw: unknown): CommandConfig {
  const d = defaultCommandConfig([]);
  const c = (raw ?? {}) as Partial<CommandConfig>;
  return {
    ...d,
    ...c,
    locales: c.locales ?? [],
    options: c.options ?? {},
    key_rule: { ...d.key_rule, ...(c.key_rule ?? {}), dot_replacement: c.key_rule?.dot_replacement ?? null },
  };
}

function defaultConfig(adapter: string, locales: Locale[]): unknown {
  if (adapter === "table") return defaultTableConfig(locales);
  if (adapter === "command") return defaultCommandConfig(locales);
  return {};
}

/** 대상의 제품 파일 확장자(파일 대화상자용). */
export function targetFileExtension(target: { adapter: string; config: unknown }): string {
  return target.adapter === "command" ? normalizeCommandConfig(target.config).file_extension || "xlsx" : "xlsx";
}

export default function ExportView({ run, projectDir }: { run: RunFn; projectDir: string | null }) {
  const [locales, setLocales] = useState<Locale[]>([]);
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [targets, setTargets] = useState<ExportTarget[]>([]);
  const [draft, setDraft] = useState<ExportTargetInput | null>(null);
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [check, setCheck] = useState<ExportCheck | null>(null);
  const [reason, setReason] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  // 내보내는 중(명령 어댑터는 외부 프로그램이 끝날 때까지 걸린다)
  const [busy, setBusy] = useState(false);

  const saved = targets.find((t) => t.id === draft?.id);
  const dirty = !!draft && !!saved && stableJson(saved) !== stableJson(draft);

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
    saved &&
    run(async () => {
      // 내보내기는 저장된 설정으로 한다
      const ext = targetFileExtension(saved);
      const path = await save({
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
        defaultPath: `${saved.name}.${ext}`,
      });
      if (typeof path !== "string") return;
      setMessage(null);
      setBusy(true);
      try {
        const r = await api.exportToFile(saved.id, path, reason.trim() || null);
        setMessage(`${r.rows}행을 내보냄. 새로 고정한 제품 키 ${r.new_keys}개${r.changeset_id ? ` (변경 묶음 #${r.changeset_id})` : ""}`);
        setReason("");
      } finally {
        setBusy(false);
      }
      await loadPreview(saved.id);
    });

  const savedCommand = saved?.adapter === "command" ? normalizeCommandConfig(saved.config).export_command : null;

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
                      config: defaultConfig(e.target.value, locales),
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
              ) : draft.adapter === "command" ? (
                <CommandConfigEditor
                  key={draft.id ?? "new"}
                  config={normalizeCommandConfig(draft.config)}
                  locales={locales}
                  projectDir={projectDir}
                  onChange={(config) => setDraft({ ...draft, config })}
                />
              ) : (
                <JsonConfigEditor
                  key={`${draft.id ?? "new"}:${draft.adapter}`}
                  value={draft.config}
                  onChange={(config) => setDraft({ ...draft, config })}
                />
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
                canExport={
                  !busy && !!check && check.errors.length === 0 && (check.warnings.length === 0 || reason.trim() !== "")
                }
                busy={busy}
                command={savedCommand}
                workDir={projectDir}
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
        <KeyRuleFields value={config} onChange={(patch) => onChange({ ...config, ...patch })} />
      </div>
    </>
  );
}

type CommonOptions = Pick<TableConfig, "key_rule" | "fallback_to_source" | "include_deprecated">;

/** 제품 키 규칙과 공통 옵션(표·명령 어댑터 공통). .field 격자 안에 넣는다. */
function KeyRuleFields({ value, onChange }: { value: CommonOptions; onChange: (patch: Partial<CommonOptions>) => void }) {
  const rule = value.key_rule;
  return (
    <>
      <span>제품 키 규칙</span>
      <div className="row">
        <input
          placeholder="접두어"
          value={rule.prefix}
          style={{ width: 100 }}
          onChange={(e) => onChange({ key_rule: { ...rule, prefix: e.target.value } })}
        />
        <select
          value={rule.case}
          onChange={(e) => onChange({ key_rule: { ...rule, case: e.target.value as TableConfig["key_rule"]["case"] } })}
        >
          <option value="keep">대소문자 그대로</option>
          <option value="upper">대문자로</option>
          <option value="lower">소문자로</option>
        </select>
        <label>
          <input
            type="checkbox"
            checked={rule.dot_replacement != null}
            onChange={(e) => onChange({ key_rule: { ...rule, dot_replacement: e.target.checked ? "_" : null } })}
          />
          '.'을 바꾸기
        </label>
        {rule.dot_replacement != null && (
          <input
            value={rule.dot_replacement}
            style={{ width: 40 }}
            onChange={(e) => onChange({ key_rule: { ...rule, dot_replacement: e.target.value } })}
          />
        )}
        <span className="muted">아직 키가 없는 토큰에만 씁니다. 한 번 내보낸 키는 바뀌지 않습니다.</span>
      </div>
      <span>옵션</span>
      <div className="row">
        <label>
          <input
            type="checkbox"
            checked={value.fallback_to_source}
            onChange={(e) => onChange({ fallback_to_source: e.target.checked })}
          />
          번역이 없으면 원문으로 채우기
        </label>
        <label>
          <input
            type="checkbox"
            checked={value.include_deprecated}
            onChange={(e) => onChange({ include_deprecated: e.target.checked })}
          />
          사용 중지(deprecated) 토큰도 내보내기
        </label>
      </div>
    </>
  );
}

/** 외부 명령 어댑터 설정. 요청·응답 형식은 docs/command-adapter.md */
function CommandConfigEditor({
  config,
  locales,
  projectDir,
  onChange,
}: {
  config: CommandConfig;
  locales: Locale[];
  projectDir: string | null;
  onChange: (c: CommandConfig) => void;
}) {
  const set = <K extends keyof CommandConfig>(k: K, v: CommandConfig[K]) => onChange({ ...config, [k]: v });
  const mono = { fontFamily: "Consolas, monospace", width: "100%" };
  const unused = locales.find((l) => !config.locales.includes(l.code));
  return (
    <div className="field">
      <span>내보내기 명령</span>
      <input
        style={mono}
        placeholder="예: python scripts/product.py export {request}"
        value={config.export_command}
        onChange={(e) => set("export_command", e.target.value)}
      />
      <span>역가져오기 명령</span>
      <input
        style={mono}
        placeholder="비우면 역가져오기를 하지 않음. 예: python scripts/product.py import {request}"
        value={config.import_command}
        onChange={(e) => set("import_command", e.target.value)}
      />
      <span />
      <div className="muted">
        {"{request}"} 자리에 요청 JSON 파일 경로가 들어갑니다. 셸 없이 실행하고, 작업 폴더는{" "}
        {projectDir ?? ".tms 파일이 있는 폴더"}입니다. 종료 코드 0이 성공입니다. 형식: docs/command-adapter.md
      </div>
      <span>언어</span>
      <div>
        {config.locales.map((code, i) => (
          <div className="row" key={i}>
            <select
              value={code}
              onChange={(e) => set("locales", config.locales.map((x, j) => (j === i ? e.target.value : x)))}
            >
              {locales.map((l) => (
                <option key={l.code} value={l.code}>
                  {l.name} ({l.code})
                </option>
              ))}
            </select>
            <button onClick={() => set("locales", config.locales.filter((_, j) => j !== i))}>빼기</button>
          </div>
        ))}
        <button
          disabled={locales.length === 0}
          onClick={() => set("locales", [...config.locales, (unused ?? locales[0]).code])}
        >
          언어 추가
        </button>
      </div>
      <span>파일 확장자</span>
      <div className="row">
        <input
          style={{ width: 80 }}
          value={config.file_extension}
          onChange={(e) => set("file_extension", e.target.value)}
        />
        <span>시간 제한</span>
        <input
          type="number"
          min={1}
          max={3600}
          style={{ width: 80 }}
          value={config.timeout_secs}
          onChange={(e) => set("timeout_secs", Number(e.target.value))}
        />
        <span>초</span>
      </div>
      <span>options (JSON)</span>
      <JsonText rows={3} value={config.options} onChange={(v) => set("options", v as Record<string, unknown>)} />
      <KeyRuleFields value={config} onChange={(patch) => onChange({ ...config, ...patch })} />
    </div>
  );
}

/** JSON 입력칸. JSON 으로 읽히면 onChange 를 부르고, 아니면 테두리를 빨갛게 한다. */
function JsonText({ value, onChange, rows }: { value: unknown; onChange: (v: unknown) => void; rows: number }) {
  const [text, setText] = useState(JSON.stringify(value, null, 2));
  const [bad, setBad] = useState(false);
  return (
    <textarea
      rows={rows}
      value={text}
      style={{ fontFamily: "monospace", width: "100%", borderColor: bad ? "#dc2626" : undefined }}
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
  );
}

/** 표·명령 어댑터가 아닌 Rust 전용 어댑터의 설정은 JSON 으로 고친다. */
function JsonConfigEditor({ value, onChange }: { value: unknown; onChange: (v: unknown) => void }) {
  return (
    <div className="field">
      <span>설정(JSON)</span>
      <JsonText rows={10} value={value} onChange={onChange} />
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
  busy,
  command,
  workDir,
  onExport,
  onFixKey,
}: {
  preview: ExportPreview;
  canExport: boolean;
  busy: boolean;
  /** 명령 어댑터면 내보낼 때 실행할 명령(남에게 받은 .tms 라면 무엇이 실행되는지 보이게) */
  command: string | null;
  workDir: string | null;
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
          {busy ? "명령 실행 중…" : "파일로 내보내기"}
        </button>
      </div>
      {command && (
        <div className="muted" style={{ marginBottom: 6 }}>
          내보낼 때 실행: <code className="cmd">{command}</code> · 작업 폴더: {workDir ?? "(알 수 없음)"}
        </div>
      )}
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
