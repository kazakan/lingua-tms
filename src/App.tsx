// 앱 뼈대: 프로젝트 열기·만들기, 화면 탭, 오류 표시. 각 화면은 views/ 에 있다.
import { useCallback, useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type ProjectInfo } from "./api";
import StringsView from "./views/StringsView";
import ImportView from "./views/ImportView";

const FILTERS = [{ name: "TMS 프로젝트", extensions: ["tms"] }];

/** 비동기 작업을 실행하고 오류를 화면 위쪽 띠에 보여 준다. */
export type RunFn = (f: () => Promise<unknown>) => Promise<void>;

const TABS = [
  { id: "strings", label: "문구" },
  { id: "import", label: "가져오기" },
] as const;
type TabId = (typeof TABS)[number]["id"];

export default function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<TabId>("strings");
  // 프로젝트를 새로 열 때마다 늘려서 화면 상태를 초기화한다.
  const [epoch, setEpoch] = useState(0);

  const run: RunFn = useCallback(async (f) => {
    try {
      setError(null);
      await f();
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    run(async () => setProject(await api.projectInfo()));
  }, [run]);

  const openFile = (create: boolean) =>
    run(async () => {
      const path = create
        ? await save({ filters: FILTERS, defaultPath: "strings.tms" })
        : await open({ filters: FILTERS, multiple: false, directory: false });
      if (typeof path !== "string") return;
      setProject(await api.openProject(path));
      setEpoch((e) => e + 1);
    });

  return (
    <div className="app">
      <header className="toolbar">
        <strong>Lingua TMS</strong>
        <button onClick={() => openFile(true)}>새 프로젝트</button>
        <button onClick={() => openFile(false)}>열기</button>
        <span className="path">{project ? project.path : "열린 프로젝트 없음"}</span>
      </header>

      {project && (
        <nav className="tabs">
          {TABS.map((t) => (
            <button key={t.id} className={t.id === tab ? "active" : ""} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </nav>
      )}

      {error && (
        <div className="error" onClick={() => setError(null)} title="눌러서 닫기">
          {error}
        </div>
      )}

      {project && (
        <main className="view" key={epoch}>
          {tab === "strings" && <StringsView run={run} />}
          {tab === "import" && <ImportView run={run} />}
        </main>
      )}
    </div>
  );
}
