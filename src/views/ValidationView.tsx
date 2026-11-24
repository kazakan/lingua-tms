// 검증 규칙(제약)과 전체 검증 결과.
// 제약 = 검증기 + 파라미터 + 범위(전역/태그/토큰) + 언어 + 심각도. 파라미터 입력 폼은 검증기의 JSON Schema로 만든다.
import { useCallback, useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import Form from "@rjsf/core";
import validator from "@rjsf/validator-ajv8";
import type { RJSFSchema } from "@rjsf/utils";
import {
  api,
  type ConstraintDef,
  type ConstraintInput,
  type Locale,
  type Scope,
  type Severity,
  type Token,
  type ValidationReport,
  type ValidatorInfo,
} from "../api";
import type { RunFn } from "../App";

const SEVERITY_LABEL: Record<Severity, string> = { error: "오류", warning: "경고", info: "정보" };
const SEVERITY_HELP = "오류: 내보내기 차단 · 경고: 사유를 적고 진행 · 정보: 표시만";

export function SeverityBadge({ s }: { s: Severity }) {
  return <span className={`badge ${s}`}>{SEVERITY_LABEL[s]}</span>;
}

/** 측정값(metrics)을 짧은 글로: {"length":12,"max":10} → length 12 · max 10 */
export function metricsText(m: unknown): string {
  if (!m || typeof m !== "object") return "";
  return Object.entries(m as Record<string, unknown>)
    .filter(([, v]) => v != null && !(Array.isArray(v) && v.length === 0))
    .map(([k, v]) => `${k} ${Array.isArray(v) ? v.join(" ") : String(v)}`)
    .join(" · ");
}

export default function ValidationView({
  run,
  onOpenCell,
}: {
  run: RunFn;
  onOpenCell: (tokenId: string, locale: string) => void;
}) {
  const [validators, setValidators] = useState<ValidatorInfo[]>([]);
  const [constraints, setConstraints] = useState<ConstraintDef[]>([]);
  const [locales, setLocales] = useState<Locale[]>([]);
  const [tokens, setTokens] = useState<Token[]>([]);
  const [tags, setTags] = useState<string[]>([]);
  const [draft, setDraft] = useState<ConstraintInput | null>(null);
  const [report, setReport] = useState<ValidationReport | null>(null);

  const reload = useCallback(async () => {
    setConstraints(await api.listConstraints());
    setTokens(await api.listTokens());
    setTags(await api.listTags());
    setReport(await api.validateAll());
  }, []);

  useEffect(() => {
    run(async () => {
      setValidators(await api.listValidators());
      setLocales(await api.listLocales());
      await reload();
    });
  }, [run, reload]);

  const tokenKey = useMemo(() => new Map(tokens.map((t) => [t.id, t.key])), [tokens]);
  const scopeText = (s: Scope) =>
    s.kind === "global" ? "전역" : s.kind === "tag" ? `태그 ${s.ref}` : `토큰 ${tokenKey.get(s.ref) ?? s.ref}`;

  const save = () =>
    draft &&
    run(async () => {
      await api.saveConstraint(draft);
      setDraft(null);
      await reload();
    });

  const remove = (c: ConstraintDef) =>
    run(async () => {
      if (!(await ask(`규칙 '${c.validator}' (${scopeText(c.scope)})을 삭제할까요?`, { title: "규칙 삭제", kind: "warning" })))
        return;
      await api.deleteConstraint(c.id);
      await reload();
    });

  const firstValidator = validators.find((v) => !v.needs_render) ?? validators[0];

  return (
    <div className="page">
      <h2>검증 규칙</h2>
      <div className="card">
        <div className="row">
          <button
            disabled={!firstValidator}
            onClick={() =>
              setDraft({
                id: null,
                validator: firstValidator!.id,
                params: {},
                scope: { kind: "global" },
                locale: null,
                severity: "error",
                enabled: true,
                description: null,
              })
            }
          >
            새 규칙
          </button>
          <span className="muted">
            같은 검증기는 가장 구체적인 범위(전역 &lt; 태그 &lt; 토큰, 같은 범위면 언어 지정)만 적용합니다. 하위 범위에서 '사용'을 끄면
            상위 규칙이 꺼집니다.
          </span>
        </div>
        {draft && (
          <ConstraintEditor
            key={draft.id ?? "new"}
            draft={draft}
            onChange={setDraft}
            onSave={save}
            onCancel={() => setDraft(null)}
            validators={validators}
            locales={locales}
            tokens={tokens}
            tags={tags}
          />
        )}
        <table className="table">
          <thead>
            <tr>
              <th>검증기</th>
              <th>범위</th>
              <th>언어</th>
              <th>심각도</th>
              <th>사용</th>
              <th>파라미터</th>
              <th>설명</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {constraints.map((c) => (
              <tr key={c.id} className={draft?.id === c.id ? "active" : ""}>
                <td>{c.validator}</td>
                <td>{scopeText(c.scope)}</td>
                <td>{c.locale ?? "전 언어"}</td>
                <td>
                  <SeverityBadge s={c.severity} />
                </td>
                <td>{c.enabled ? "켬" : "끔"}</td>
                <td className="muted">{metricsText(c.params)}</td>
                <td>{c.description}</td>
                <td style={{ whiteSpace: "nowrap" }}>
                  <button onClick={() => setDraft({ ...c })}>수정</button>{" "}
                  <button className="danger" onClick={() => remove(c)}>
                    삭제
                  </button>
                </td>
              </tr>
            ))}
            {constraints.length === 0 && (
              <tr>
                <td colSpan={8} className="muted">
                  규칙이 없습니다
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <h2>검증 결과</h2>
      <div className="card">
        <div className="row">
          <button onClick={() => run(async () => setReport(await api.validateAll()))}>전체 검증 다시 실행</button>
          <span className="muted">{SEVERITY_HELP}</span>
        </div>
        {report && <ReportView report={report} onOpenCell={onOpenCell} />}
      </div>
    </div>
  );
}

function ReportView({
  report,
  onOpenCell,
}: {
  report: ValidationReport;
  onOpenCell: (tokenId: string, locale: string) => void;
}) {
  const s = report.summary;
  const rows = report.cells.flatMap((c) => c.findings.map((f, i) => ({ c, f, i })));
  return (
    <>
      <div className="stats">
        <span>
          셀 <b>{s.cells}</b> · 검사 <b>{s.checks}</b>
        </span>
        <span>
          오류 <b>{s.error}</b>
        </span>
        <span>
          경고 <b>{s.warning}</b>
        </span>
        <span>
          정보 <b>{s.info}</b>
        </span>
        {s.broken > 0 && (
          <span className="notice">
            검증기 오류 <b>{s.broken}</b>
          </span>
        )}
        {s.skipped_render > 0 && (
          <span className="muted">렌더 검사 {s.skipped_render}건은 측정 엔진 연결 전이라 건너뜀</span>
        )}
      </div>
      {rows.length > 0 && (
        <div className="table-scroll">
          <table className="table">
            <thead>
              <tr>
                <th>토큰</th>
                <th>언어</th>
                <th>검증기</th>
                <th>심각도</th>
                <th>내용</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ c, f, i }) => (
                <tr
                  key={`${c.token_id}|${c.locale}|${i}`}
                  className="clickable"
                  title="문구 표에서 이 셀 열기"
                  onClick={() => onOpenCell(c.token_id, c.locale)}
                >
                  <td>{c.token_key}</td>
                  <td>{c.locale}</td>
                  <td>{f.validator}</td>
                  <td>
                    <SeverityBadge s={f.severity} />
                  </td>
                  <td>
                    {f.error ? (
                      <span className="notice">검증기 오류: {f.error}</span>
                    ) : (
                      <>
                        {f.outcome?.message} <span className="muted">{metricsText(f.outcome?.metrics)}</span>
                      </>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function ConstraintEditor(props: {
  draft: ConstraintInput;
  onChange: (d: ConstraintInput) => void;
  onSave: () => void;
  onCancel: () => void;
  validators: ValidatorInfo[];
  locales: Locale[];
  tokens: Token[];
  tags: string[];
}) {
  const { draft, onChange, validators, locales, tokens, tags } = props;
  const v = validators.find((x) => x.id === draft.validator);
  const schema = (v?.params_schema ?? { type: "object" }) as RJSFSchema;
  const hasParams = Object.keys((schema.properties as object | undefined) ?? {}).length > 0;
  const tokenKey = tokens.find((t) => draft.scope.kind === "token" && t.id === draft.scope.ref)?.key ?? "";
  const [tokenText, setTokenText] = useState(tokenKey);

  return (
    <div className="card" style={{ background: "#f8fafc" }}>
      <div className="field">
        <span>검증기</span>
        <select value={draft.validator} onChange={(e) => onChange({ ...draft, validator: e.target.value, params: {} })}>
          {validators.map((x) => (
            <option key={x.id} value={x.id}>
              {x.id} — {x.description}
              {x.needs_render ? " (렌더: 측정 엔진 연결 전까지 건너뜀)" : ""}
            </option>
          ))}
        </select>
        <span>범위</span>
        <div className="row" style={{ margin: 0 }}>
          <select
            value={draft.scope.kind}
            onChange={(e) => {
              const kind = e.target.value as Scope["kind"];
              onChange({ ...draft, scope: kind === "global" ? { kind } : { kind, ref: "" } });
              setTokenText("");
            }}
          >
            <option value="global">전역</option>
            <option value="tag">태그</option>
            <option value="token">토큰</option>
          </select>
          {draft.scope.kind === "tag" && (
            <>
              <input
                list="tag-list"
                placeholder="태그 이름"
                value={draft.scope.ref}
                onChange={(e) => onChange({ ...draft, scope: { kind: "tag", ref: e.target.value } })}
              />
              <datalist id="tag-list">
                {tags.map((t) => (
                  <option key={t} value={t} />
                ))}
              </datalist>
            </>
          )}
          {draft.scope.kind === "token" && (
            <>
              <input
                list="token-list"
                placeholder="토큰명"
                value={tokenText}
                onChange={(e) => {
                  setTokenText(e.target.value);
                  const t = tokens.find((x) => x.key === e.target.value);
                  onChange({ ...draft, scope: { kind: "token", ref: t?.id ?? "" } });
                }}
              />
              <datalist id="token-list">
                {tokens.map((t) => (
                  <option key={t.id} value={t.key} />
                ))}
              </datalist>
              {tokenText && draft.scope.ref === "" && <span className="notice">없는 토큰명</span>}
            </>
          )}
        </div>
        <span>언어</span>
        <select value={draft.locale ?? ""} onChange={(e) => onChange({ ...draft, locale: e.target.value || null })}>
          <option value="">전 언어</option>
          {locales.map((l) => (
            <option key={l.code} value={l.code}>
              {l.name} ({l.code})
            </option>
          ))}
        </select>
        <span>심각도</span>
        <div className="row" style={{ margin: 0 }}>
          <select value={draft.severity} onChange={(e) => onChange({ ...draft, severity: e.target.value as Severity })}>
            <option value="error">오류</option>
            <option value="warning">경고</option>
            <option value="info">정보</option>
          </select>
          <span className="muted">{SEVERITY_HELP}</span>
        </div>
        <span>사용</span>
        <label>
          <input type="checkbox" checked={draft.enabled} onChange={(e) => onChange({ ...draft, enabled: e.target.checked })} />
          켬(끄면 이 범위에서 같은 검증기의 상위 규칙도 꺼짐)
        </label>
        <span>설명</span>
        <input value={draft.description ?? ""} onChange={(e) => onChange({ ...draft, description: e.target.value || null })} />
        <span>파라미터</span>
        {hasParams ? (
          <div className="rjsf">
            <Form
              key={draft.validator}
              schema={schema}
              formData={draft.params}
              validator={validator}
              onChange={(e) => onChange({ ...draft, params: (e.formData ?? {}) as Record<string, unknown> })}
              uiSchema={{ "ui:submitButtonOptions": { norender: true } }}
              showErrorList={false}
            />
          </div>
        ) : (
          <span className="muted">없음</span>
        )}
      </div>
      <div className="row">
        <button className="primary" onClick={props.onSave}>
          저장
        </button>
        <button onClick={props.onCancel}>취소</button>
      </div>
    </div>
  );
}
