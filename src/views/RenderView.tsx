// 렌더 측정 설정: 엔진 DLL, 폰트, 렌더 프로필(엔진 + 스타일 + 언어별 폰트 폴백 순서).
// 폭·줄 수는 엔진 DLL 이 잰다. 검증 탭의 렌더 규칙(render.*)은 여기서 만든 프로필을 골라 쓴다.
import { useCallback, useEffect, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  DEFAULT_FONTS,
  type EngineInfo,
  type FontAsset,
  type LayoutView,
  type Locale,
  type RenderProfile,
  type RenderProfileInput,
} from "../api";
import type { RunFn } from "../App";
import MeasureResult from "../components/MeasureResult";

const kb = (n: number) => (n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(n / 1024)} KB`);

export default function RenderView({ run }: { run: RunFn }) {
  const [engines, setEngines] = useState<EngineInfo[]>([]);
  const [fonts, setFonts] = useState<FontAsset[]>([]);
  const [profiles, setProfiles] = useState<RenderProfile[]>([]);
  const [locales, setLocales] = useState<Locale[]>([]);
  const [draft, setDraft] = useState<RenderProfileInput | null>(null);

  const reload = useCallback(async () => {
    setEngines(await api.listEngines());
    setFonts(await api.listFonts());
    setProfiles(await api.listRenderProfiles());
  }, []);

  useEffect(() => {
    run(async () => {
      setLocales(await api.listLocales());
      await reload();
    });
  }, [run, reload]);

  const fontName = (id: string) => fonts.find((f) => f.id === id)?.family ?? "(없는 폰트)";

  const registerEngine = () =>
    run(async () => {
      const p = await open({ filters: [{ name: "엔진 DLL", extensions: ["dll"] }], multiple: false, directory: false });
      if (typeof p !== "string") return;
      await api.registerEngine(p, null);
      await reload();
    });

  const removeEngine = (e: EngineInfo) =>
    run(async () => {
      if (!(await ask(`엔진 ${e.id} 등록을 지울까요? DLL 파일은 지우지 않습니다.`, { title: "엔진 삭제", kind: "warning" })))
        return;
      await api.deleteEngine(e.id);
      await reload();
    });

  const addFonts = () =>
    run(async () => {
      const picked = await open({
        filters: [{ name: "폰트", extensions: ["ttf", "otf", "ttc"] }],
        multiple: true,
        directory: false,
      });
      const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      if (paths.length === 0) return;
      await api.addFonts(paths);
      await reload();
    });

  const removeFont = (f: FontAsset) =>
    run(async () => {
      if (!(await ask(`폰트 '${f.family}'를 프로젝트에서 지울까요?`, { title: "폰트 삭제", kind: "warning" }))) return;
      await api.deleteFont(f.id);
      await reload();
    });

  const newProfile = () =>
    setDraft({
      id: null,
      name: "",
      engine_id: engines[0]?.id ?? "",
      size_px: 16,
      letter_spacing: 0,
      line_height: 0,
      wrap: "engine",
      fonts: { [DEFAULT_FONTS]: fonts[0] ? [fonts[0].id] : [] },
    });

  const saveProfile = () =>
    draft &&
    run(async () => {
      const id = await api.saveRenderProfile(draft);
      setDraft({ ...draft, id });
      await reload();
    });

  const removeProfile = (p: RenderProfile) =>
    run(async () => {
      if (!(await ask(`렌더 프로필 '${p.name}'을 지울까요?`, { title: "프로필 삭제", kind: "warning" }))) return;
      await api.deleteRenderProfile(p.id);
      if (draft?.id === p.id) setDraft(null);
      await reload();
    });

  const canMakeProfile = engines.length > 0 && fonts.length > 0;

  return (
    <div className="page">
      <h2>렌더 측정</h2>
      <p className="muted" style={{ marginTop: -6 }}>
        폭·줄 수는 엔진 DLL이 잽니다. 검증 탭의 렌더 규칙(render.*)은 여기서 만든 프로필을 골라 씁니다. 이 PC에서 DLL을 쓸 수
        없으면 렌더 검사는 건너뛰고, 내보낼 때 사유를 받습니다.
      </p>

      <div className="card">
        <h3>측정 엔진 DLL</h3>
        <div className="row">
          <button onClick={registerEngine}>DLL 등록</button>
          <span className="muted">
            DLL이 알려 주는 엔진 버전이 이름이 됩니다. 같은 버전을 다시 등록하면 경로만 바뀝니다. .tms 폴더 안의 DLL은 상대 경로로
            저장합니다.
          </span>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>엔진 버전</th>
              <th>경로</th>
              <th>상태</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {engines.map((e) => (
              <tr key={e.id}>
                <td>{e.id}</td>
                <td title={e.resolved_path}>
                  <code className="cmd">{e.dll_path}</code>
                </td>
                <td>
                  {e.loaded ? (
                    <span className="badge">불러옴</span>
                  ) : e.exists ? (
                    <span className="badge gray">파일 있음</span>
                  ) : (
                    <span className="badge error">파일 없음</span>
                  )}
                </td>
                <td>
                  <button className="danger" onClick={() => removeEngine(e)}>
                    삭제
                  </button>
                </td>
              </tr>
            ))}
            {engines.length === 0 && (
              <tr>
                <td colSpan={4} className="muted">
                  등록한 엔진이 없습니다
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <h3>폰트</h3>
        <div className="row">
          <button onClick={addFonts}>폰트 추가</button>
          <span className="muted">폰트 파일은 프로젝트(.tms) 안에 들어갑니다. 같은 내용의 파일은 한 번만 들어갑니다.</span>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>이름</th>
              <th>파일</th>
              <th>크기</th>
              <th>sha256</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {fonts.map((f) => (
              <tr key={f.id}>
                <td>{f.family}</td>
                <td>{f.file_name}</td>
                <td>{kb(f.size)}</td>
                <td className="muted" title={f.sha256}>
                  {f.sha256.slice(0, 12)}…
                </td>
                <td>
                  <button className="danger" onClick={() => removeFont(f)}>
                    삭제
                  </button>
                </td>
              </tr>
            ))}
            {fonts.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  폰트가 없습니다
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <h3>렌더 프로필</h3>
        <div className="row">
          <button onClick={newProfile} disabled={!canMakeProfile}>
            새 프로필
          </button>
          {!canMakeProfile && <span className="notice">엔진 DLL과 폰트를 먼저 등록하세요</span>}
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>이름</th>
              <th>엔진</th>
              <th>크기</th>
              <th>자간</th>
              <th>줄 높이</th>
              <th>줄바꿈</th>
              <th>폰트(폴백 순서)</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {profiles.map((p) => (
              <tr key={p.id} className={draft?.id === p.id ? "active" : ""}>
                <td>{p.name}</td>
                <td>{p.engine_id}</td>
                <td>{p.size_px}px</td>
                <td>{p.letter_spacing}px</td>
                <td>{p.line_height === 0 ? "엔진 기본" : `${p.line_height}px`}</td>
                <td>{p.wrap === "engine" ? "엔진 기본" : "없음"}</td>
                <td>
                  {Object.entries(p.fonts).map(([k, ids]) => (
                    <div key={k}>
                      <span className="muted">{k === DEFAULT_FONTS ? "기본" : k}:</span> {ids.map(fontName).join(" → ")}
                    </div>
                  ))}
                </td>
                <td style={{ whiteSpace: "nowrap" }}>
                  <button onClick={() => setDraft({ ...p })}>수정</button>{" "}
                  <button className="danger" onClick={() => removeProfile(p)}>
                    삭제
                  </button>
                </td>
              </tr>
            ))}
            {profiles.length === 0 && (
              <tr>
                <td colSpan={8} className="muted">
                  프로필이 없습니다
                </td>
              </tr>
            )}
          </tbody>
        </table>
        {draft && (
          <ProfileEditor
            draft={draft}
            onChange={setDraft}
            onSave={saveProfile}
            onClose={() => setDraft(null)}
            engines={engines}
            fonts={fonts}
            locales={locales}
          />
        )}
      </div>
    </div>
  );
}

function ProfileEditor(props: {
  draft: RenderProfileInput;
  onChange: (d: RenderProfileInput) => void;
  onSave: () => void;
  onClose: () => void;
  engines: EngineInfo[];
  fonts: FontAsset[];
  locales: Locale[];
}) {
  const { draft, onChange, engines, fonts, locales } = props;
  const num = (v: string) => (v.trim() === "" ? 0 : Number(v));
  const setFonts = (key: string, ids: string[] | null) => {
    const next = { ...draft.fonts };
    if (ids == null) delete next[key];
    else next[key] = ids;
    onChange({ ...draft, fonts: next });
  };
  const overrides = locales.filter((l) => draft.fonts[l.code] !== undefined);
  const free = locales.filter((l) => draft.fonts[l.code] === undefined);

  return (
    <div className="card" style={{ background: "#f8fafc" }}>
      <div className="field">
        <span>이름</span>
        <input value={draft.name} onChange={(e) => onChange({ ...draft, name: e.target.value })} placeholder="버튼 16px" />
        <span>엔진</span>
        <select value={draft.engine_id} onChange={(e) => onChange({ ...draft, engine_id: e.target.value })}>
          {engines.map((e) => (
            <option key={e.id} value={e.id}>
              {e.id}
            </option>
          ))}
        </select>
        <span>글자 크기(px)</span>
        <input type="number" min={1} step={0.5} value={draft.size_px} onChange={(e) => onChange({ ...draft, size_px: num(e.target.value) })} />
        <span>자간(px)</span>
        <input
          type="number"
          step={0.1}
          value={draft.letter_spacing}
          onChange={(e) => onChange({ ...draft, letter_spacing: num(e.target.value) })}
        />
        <span>줄 높이(px)</span>
        <div className="row" style={{ margin: 0 }}>
          <input
            type="number"
            min={0}
            step={0.5}
            value={draft.line_height}
            onChange={(e) => onChange({ ...draft, line_height: num(e.target.value) })}
          />
          <span className="muted">0 = 엔진 기본값</span>
        </div>
        <span>줄바꿈</span>
        <div className="row" style={{ margin: 0 }}>
          <select value={draft.wrap} onChange={(e) => onChange({ ...draft, wrap: e.target.value as RenderProfileInput["wrap"] })}>
            <option value="engine">엔진 기본 줄바꿈</option>
            <option value="none">줄바꿈 없음</option>
          </select>
          <span className="muted">줄 수·넘침 규칙은 항상 박스 폭에서 줄바꿈해 잽니다</span>
        </div>
        <span>기본 폰트</span>
        <FontList ids={draft.fonts[DEFAULT_FONTS] ?? []} fonts={fonts} onChange={(ids) => setFonts(DEFAULT_FONTS, ids)} />
        {overrides.map((l) => (
          <LocaleFonts
            key={l.code}
            locale={l}
            ids={draft.fonts[l.code]}
            fonts={fonts}
            onChange={(ids) => setFonts(l.code, ids)}
            onRemove={() => setFonts(l.code, null)}
          />
        ))}
        <span>언어별 폰트</span>
        <div className="row" style={{ margin: 0 }}>
          <select
            value=""
            disabled={free.length === 0}
            onChange={(e) => e.target.value && setFonts(e.target.value, [...(draft.fonts[DEFAULT_FONTS] ?? [])])}
          >
            <option value="">언어 추가…</option>
            {free.map((l) => (
              <option key={l.code} value={l.code}>
                {l.name} ({l.code})
              </option>
            ))}
          </select>
          <span className="muted">언어 목록이 있으면 그 언어는 기본 폰트 대신 그 목록만 씁니다(위에서부터 폴백)</span>
        </div>
      </div>
      <div className="row">
        <button className="primary" onClick={props.onSave}>
          저장
        </button>
        <button onClick={props.onClose}>닫기</button>
      </div>
      <PreviewBox draft={draft} locales={locales} />
    </div>
  );
}

function LocaleFonts(props: {
  locale: Locale;
  ids: string[];
  fonts: FontAsset[];
  onChange: (ids: string[]) => void;
  onRemove: () => void;
}) {
  return (
    <>
      <span>
        {props.locale.name} ({props.locale.code}){" "}
        <button className="danger" onClick={props.onRemove} title="이 언어도 기본 폰트를 쓰게">
          ✕
        </button>
      </span>
      <FontList ids={props.ids} fonts={props.fonts} onChange={props.onChange} />
    </>
  );
}

/** 폰트 폴백 순서 편집: 위에서부터 찾는다. */
function FontList({ ids, fonts, onChange }: { ids: string[]; fonts: FontAsset[]; onChange: (ids: string[]) => void }) {
  const name = (id: string) => fonts.find((f) => f.id === id)?.family ?? "(없는 폰트)";
  const move = (i: number, d: -1 | 1) => {
    const next = [...ids];
    [next[i], next[i + d]] = [next[i + d], next[i]];
    onChange(next);
  };
  const rest = fonts.filter((f) => !ids.includes(f.id));
  return (
    <div>
      <ol className="font-list">
        {ids.map((id, i) => (
          <li key={id}>
            {name(id)}{" "}
            <button disabled={i === 0} onClick={() => move(i, -1)} title="위로">
              ▲
            </button>
            <button disabled={i === ids.length - 1} onClick={() => move(i, 1)} title="아래로">
              ▼
            </button>
            <button className="danger" onClick={() => onChange(ids.filter((x) => x !== id))} title="빼기">
              ✕
            </button>
          </li>
        ))}
      </ol>
      <select value="" disabled={rest.length === 0} onChange={(e) => e.target.value && onChange([...ids, e.target.value])}>
        <option value="">폰트 추가…</option>
        {rest.map((f) => (
          <option key={f.id} value={f.id}>
            {f.family} ({f.file_name})
          </option>
        ))}
      </select>
    </div>
  );
}

/** 편집 중인(저장 전) 프로필로 엔진이 잰 결과를 바로 보여 준다. */
function PreviewBox({ draft, locales }: { draft: RenderProfileInput; locales: Locale[] }) {
  const [locale, setLocale] = useState(locales.find((l) => l.is_source)?.code ?? locales[0]?.code ?? "");
  const [text, setText] = useState("확인 버튼을 눌러 주세요");
  const [boxWidth, setBoxWidth] = useState(0);
  const [result, setResult] = useState<LayoutView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const key = JSON.stringify(draft);

  useEffect(() => {
    if (!locale) return;
    let alive = true;
    const t = setTimeout(() => {
      api
        .measureText(JSON.parse(key) as RenderProfileInput, locale, text, boxWidth)
        .then((r) => alive && (setResult(r), setError(null)))
        .catch((e) => alive && (setResult(null), setError(String(e))));
    }, 200);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [key, locale, text, boxWidth]);

  return (
    <div>
      <h4>미리 보기</h4>
      <div className="row">
        <select value={locale} onChange={(e) => setLocale(e.target.value)}>
          {locales.map((l) => (
            <option key={l.code} value={l.code}>
              {l.name} ({l.code})
            </option>
          ))}
        </select>
        <label>
          박스 폭(px)
          <input type="number" min={0} value={boxWidth} onChange={(e) => setBoxWidth(Math.max(0, Number(e.target.value) || 0))} style={{ width: 90 }} />
        </label>
        <span className="muted">0 = 줄바꿈 없이 한 줄</span>
      </div>
      <textarea value={text} onChange={(e) => setText(e.target.value)} rows={2} style={{ width: "100%" }} />
      {error && <div className="notice">{error}</div>}
      {result && (
        <>
          <MeasureResult layout={result} />
          <div className="muted">엔진 {result.engine}</div>
        </>
      )}
    </div>
  );
}
