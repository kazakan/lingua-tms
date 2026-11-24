// 뼈대 화면: 프로젝트 열기/만들기, 토큰·언어 추가, 문구 표 편집, 셀 이력.
// 표는 1,600행 정도까지 일반 <table>로 버틴다. 이후 Glide Data Grid 로 교체 예정(CLAUDE.md 참고).
import { useCallback, useEffect, useState, type KeyboardEvent } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type Grid, type ProjectInfo, type RevRow } from "./api";

const FILTERS = [{ name: "TMS 프로젝트", extensions: ["tms"] }];

type Selected = { tokenId: string; tokenKey: string; locale: string };

export default function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [grid, setGrid] = useState<Grid | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Selected | null>(null);
  const [history, setHistory] = useState<RevRow[]>([]);

  const run = useCallback(async (f: () => Promise<unknown>) => {
    try {
      setError(null);
      await f();
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const reload = useCallback(() => run(async () => setGrid(await api.loadGrid())), [run]);

  useEffect(() => {
    run(async () => {
      const info = await api.projectInfo();
      setProject(info);
      if (info) setGrid(await api.loadGrid());
    });
  }, [run]);

  useEffect(() => {
    if (!selected) return setHistory([]);
    run(async () => setHistory(await api.cellHistory(selected.tokenId, selected.locale)));
  }, [selected, grid, run]);

  const openFile = (create: boolean) =>
    run(async () => {
      const path = create
        ? await save({ filters: FILTERS, defaultPath: "strings.tms" })
        : await open({ filters: FILTERS, multiple: false, directory: false });
      if (typeof path !== "string") return;
      setProject(await api.openProject(path));
      setSelected(null);
      setGrid(await api.loadGrid());
    });

  return (
    <div className="app">
      <header className="toolbar">
        <strong>Lingua TMS</strong>
        <button onClick={() => openFile(true)}>새 프로젝트</button>
        <button onClick={() => openFile(false)}>열기</button>
        <span className="path">{project ? project.path : "열린 프로젝트 없음"}</span>
      </header>

      {error && (
        <div className="error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      {project && grid && (
        <>
          <AddForms run={run} reload={reload} hasLocales={grid.locales.length > 0} />
          <main className="body">
            <StringTable grid={grid} run={run} reload={reload} selected={selected} onSelect={setSelected} />
            <aside className="side">
              <h3>셀 이력</h3>
              {selected ? (
                <>
                  <div className="muted">
                    {selected.tokenKey} · {selected.locale}
                  </div>
                  <ol className="history">
                    {history.map((h) => (
                      <li key={h.changeset_id}>
                        <div className="muted">
                          #{h.changeset_id} {h.op} · {h.author} · {h.created_at}
                        </div>
                        <div>
                          <del>{h.old?.value ?? "∅"}</del> → <ins>{h.new?.value ?? "∅"}</ins>
                        </div>
                      </li>
                    ))}
                  </ol>
                </>
              ) : (
                <div className="muted">셀을 선택하세요</div>
              )}
            </aside>
          </main>
        </>
      )}
    </div>
  );
}

type RunFn = (f: () => Promise<unknown>) => Promise<void>;

function AddForms({ run, reload, hasLocales }: { run: RunFn; reload: () => void; hasLocales: boolean }) {
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
            reload();
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
            reload();
          });
        }}
      >
        <input placeholder="토큰명 (home.title)" value={key} onChange={(e) => setKey(e.target.value)} required />
        <button type="submit">토큰 추가</button>
      </form>
    </div>
  );
}

function StringTable(props: {
  grid: Grid;
  run: RunFn;
  reload: () => void;
  selected: Selected | null;
  onSelect: (s: Selected) => void;
}) {
  const { grid, run, reload, selected, onSelect } = props;
  if (grid.locales.length === 0) return <div className="empty">언어를 먼저 추가하세요</div>;

  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>토큰</th>
            {grid.locales.map((l) => (
              <th key={l.code}>
                {l.name} <span className="muted">{l.code}</span>
                {l.is_source && <span className="badge">원문</span>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {grid.rows.map((row) => (
            <tr key={row.token.id}>
              <td>
                <EditableText
                  value={row.token.key}
                  onCommit={(v) =>
                    run(async () => {
                      await api.renameToken(row.token.id, v);
                      reload();
                    })
                  }
                />
              </td>
              {grid.locales.map((l) => {
                const cell = row.cells[l.code];
                const isSel = selected?.tokenId === row.token.id && selected.locale === l.code;
                const cls = [cell?.outdated ? "outdated" : "", cell ? "" : "missing", isSel ? "selected" : ""].join(" ");
                return (
                  <td
                    key={l.code}
                    className={cls}
                    onFocus={() => onSelect({ tokenId: row.token.id, tokenKey: row.token.key, locale: l.code })}
                  >
                    <EditableText
                      value={cell?.value ?? ""}
                      multiline
                      onCommit={(v) =>
                        run(async () => {
                          await api.setCell(row.token.id, l.code, v);
                          reload();
                        })
                      }
                    />
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** 포커스를 잃을 때 값이 바뀌었으면 커밋. Esc 는 취소, Enter 는 커밋(여러 줄은 Shift+Enter 로 줄바꿈). */
function EditableText({ value, onCommit, multiline }: { value: string; onCommit: (v: string) => void; multiline?: boolean }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
    if (e.key === "Escape") {
      setDraft(value);
      e.currentTarget.blur();
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      e.currentTarget.blur();
    }
  };

  return multiline ? (
    <textarea
      rows={1}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={onKeyDown}
    />
  ) : (
    <input value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={commit} onKeyDown={onKeyDown} />
  );
}
