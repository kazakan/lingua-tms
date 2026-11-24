// 이력: 변경 묶음 목록(되돌리기·롤백·릴리스), 두 시점 비교, 파일 두 개 비교.
// 롤백·되돌리기도 과거 값을 새 변경 묶음으로 쓴다. 이력 행은 지우거나 고치지 않는다.
import { useCallback, useEffect, useMemo, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  type ChangesetInfo,
  type ColumnMapping,
  type ImportSource,
  type KeyedDiff,
  type Locale,
  type Release,
  type SheetPreview,
  type ValueDiff,
} from "../api";
import type { RunFn } from "../App";
import Change from "../components/Change";
import MappingEditor, { mappingReady, suggestMapping } from "../components/MappingEditor";
import { SHEET_FILTERS } from "./ImportView";

const KIND_LABEL: Record<ChangesetInfo["kind"], string> = {
  edit: "편집",
  import: "가져오기",
  revert: "되돌리기",
  rename: "이름 변경",
  admin: "관리",
};
const LIST_LIMIT = 500;
const MAX_ROWS_SHOWN = 1000;

type OpenCell = (tokenId: string, locale: string) => void;

export default function HistoryView({ run, onOpenCell }: { run: RunFn; onOpenCell: OpenCell }) {
  const [sub, setSub] = useState<"changesets" | "points" | "files">("changesets");
  return (
    <div className="strings">
      <div className="filters">
        {(
          [
            ["changesets", "변경 이력"],
            ["points", "두 시점 비교"],
            ["files", "파일 두 개 비교"],
          ] as const
        ).map(([id, label]) => (
          <label key={id}>
            <input type="radio" checked={sub === id} onChange={() => setSub(id)} />
            {label}
          </label>
        ))}
      </div>
      {sub === "changesets" && <ChangesetsPanel run={run} onOpenCell={onOpenCell} />}
      {sub === "points" && <PointsPanel run={run} onOpenCell={onOpenCell} />}
      {sub === "files" && <FilesPanel run={run} />}
    </div>
  );
}

// ── 변경 이력 ─────────────────────────────

function ChangesetsPanel({ run, onOpenCell }: { run: RunFn; onOpenCell: OpenCell }) {
  const [list, setList] = useState<ChangesetInfo[]>([]);
  const [selected, setSelected] = useState<ChangesetInfo | null>(null);
  const [cells, setCells] = useState<KeyedDiff[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [releaseName, setReleaseName] = useState("");
  const [releaseNote, setReleaseNote] = useState("");

  const reload = useCallback(async () => {
    const l = await api.listChangesets(LIST_LIMIT);
    setList(l);
    return l;
  }, []);

  useEffect(() => {
    run(reload);
  }, [run, reload]);

  const select = (c: ChangesetInfo) =>
    run(async () => {
      setSelected(c);
      setMessage(null);
      setCells(c.cell_changes > 0 ? await api.diffChangesets(c.id - 1, c.id) : []);
    });

  const afterChange = async (note: string) => {
    const l = await reload();
    setMessage(note);
    const again = l.find((x) => x.id === selected?.id);
    if (again) setSelected(again);
  };

  const revert = (c: ChangesetInfo) =>
    run(async () => {
      if (!(await ask(`변경 묶음 #${c.id}의 문구 변경 ${c.cell_changes}칸을 되돌릴까요?\n그 뒤에 다시 바뀐 칸은 건너뜁니다.`, { title: "되돌리기" })))
        return;
      const out = await api.revertChangeset(c.id);
      const tokens = new Map((await api.listTokens()).map((t) => [t.id, t.key]));
      const conflicts = out.conflicts.map((k) => `${tokens.get(k.token_id) ?? k.token_id} · ${k.locale}`);
      await afterChange(
        (out.changeset_id ? `변경 묶음 #${out.changeset_id}로 되돌림.` : "되돌릴 칸이 없음.") +
          (conflicts.length ? ` 나중에 다시 바뀌어 건너뛴 칸 ${conflicts.length}개: ${conflicts.join(", ")}` : ""),
      );
    });

  const rollback = (c: ChangesetInfo) =>
    run(async () => {
      if (
        !(await ask(`전체 문구를 변경 묶음 #${c.id} 시점으로 되돌릴까요?\n그 뒤의 문구 변경이 모두 새 변경 묶음 하나로 되돌려집니다(이력은 남음).`, {
          title: "전체 롤백",
          kind: "warning",
        }))
      )
        return;
      const id = await api.rollbackTo(c.id);
      await afterChange(id ? `변경 묶음 #${id}로 #${c.id} 시점까지 롤백함` : "이미 그 시점과 같음");
    });

  const makeRelease = (c: ChangesetInfo) =>
    run(async () => {
      await api.createRelease(releaseName, c.id, releaseNote || null);
      setReleaseName("");
      setReleaseNote("");
      await afterChange(`릴리스 '${releaseName.trim()}'을 #${c.id}에 붙임`);
    });

  return (
    <div className="split">
      <div className="list">
        {list.map((c) => (
          <div key={c.id} className={`list-item ${c.id === selected?.id ? "active" : ""}`} onClick={() => select(c)}>
            <div>
              #{c.id} <span className="badge gray">{KIND_LABEL[c.kind]}</span>
              {c.releases.map((r) => (
                <span key={r} className="badge">
                  {r}
                </span>
              ))}
            </div>
            <div className="muted">
              {c.author} · {c.created_at.replace("T", " ").slice(0, 19)}
            </div>
            <div className="muted" style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
              {c.cell_changes > 0 && `칸 ${c.cell_changes} · `}
              {c.message}
            </div>
          </div>
        ))}
        {list.length === LIST_LIMIT && <div className="list-item muted">최근 {LIST_LIMIT}개만 표시</div>}
      </div>
      <div className="detail">
        {!selected ? (
          <div className="muted">왼쪽에서 변경 묶음을 고르세요.</div>
        ) : (
          <>
            <div className="card">
              <h3>
                #{selected.id} {KIND_LABEL[selected.kind]}
                {selected.releases.map((r) => (
                  <span key={r} className="badge">
                    {r}
                  </span>
                ))}
              </h3>
              <div className="muted">
                {selected.author} · {selected.created_at}
                {selected.reverts_id != null && ` · #${selected.reverts_id} 되돌리기`}
              </div>
              {selected.message && <div className="pre">{selected.message}</div>}
              <div className="row" style={{ marginTop: 8 }}>
                <button disabled={selected.cell_changes === 0} onClick={() => revert(selected)}>
                  이 변경 되돌리기
                </button>
                <button onClick={() => rollback(selected)}>이 시점으로 전체 롤백</button>
              </div>
              <form
                className="row"
                onSubmit={(e) => {
                  e.preventDefault();
                  makeRelease(selected);
                }}
              >
                <input placeholder="릴리스 이름(v1.2 등)" value={releaseName} onChange={(e) => setReleaseName(e.target.value)} />
                <input placeholder="메모" value={releaseNote} onChange={(e) => setReleaseNote(e.target.value)} />
                <button type="submit" disabled={!releaseName.trim()}>
                  여기에 릴리스 만들기
                </button>
              </form>
              {message && <div className="notice">{message}</div>}
            </div>
            {selected.cell_changes > 0 ? (
              <DiffTable rows={cells} onOpenCell={onOpenCell} />
            ) : (
              <div className="muted">문구 변경이 없는 변경 묶음입니다(엔티티 변경 {selected.entity_changes}건).</div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

// ── 두 시점 비교 ──────────────────────────

function PointsPanel({ run, onOpenCell }: { run: RunFn; onOpenCell: OpenCell }) {
  const [list, setList] = useState<ChangesetInfo[]>([]);
  const [releases, setReleases] = useState<Release[]>([]);
  const [from, setFrom] = useState(0);
  const [to, setTo] = useState(0);
  const [rows, setRows] = useState<KeyedDiff[] | null>(null);

  useEffect(() => {
    run(async () => {
      const l = await api.listChangesets(LIST_LIMIT);
      const r = await api.listReleases();
      setList(l);
      setReleases(r);
      setTo(l[0]?.id ?? 0);
      setFrom(r[0]?.changeset_id ?? 0);
    });
  }, [run]);

  const options = useMemo(
    () => [
      ...(list[0] ? [{ value: list[0].id, label: `현재 (#${list[0].id})` }] : []),
      ...releases.map((r) => ({ value: r.changeset_id, label: `릴리스 ${r.name} (#${r.changeset_id})` })),
      ...list.map((c) => ({ value: c.id, label: `#${c.id} ${KIND_LABEL[c.kind]} ${c.message ?? ""}`.slice(0, 80) })),
      { value: 0, label: "처음(빈 상태)" },
    ],
    [list, releases],
  );
  const select = (value: number, onChange: (v: number) => void) => (
    <select value={value} onChange={(e) => onChange(Number(e.target.value))}>
      {options.map((o, i) => (
        <option key={i} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );

  return (
    <div className="page">
      <div className="card">
        <div className="row">
          기준 {select(from, setFrom)} → 대상 {select(to, setTo)}
          <button onClick={() => run(async () => setRows(await api.diffChangesets(from, to)))}>비교</button>
        </div>
        <div className="muted">두 시점 사이 구간의 이력으로 계산합니다. 처음 값과 마지막 값이 같아진 칸은 나오지 않습니다.</div>
      </div>
      {rows && <DiffTable rows={rows} onOpenCell={onOpenCell} />}
    </div>
  );
}

function DiffTable({ rows, onOpenCell }: { rows: KeyedDiff[]; onOpenCell: OpenCell }) {
  const added = rows.filter((r) => r.old == null).length;
  const removed = rows.filter((r) => r.new == null).length;
  return (
    <div className="card">
      <div className="stats">
        <span>
          바뀐 칸 <b>{rows.length}</b> (새 값 {added} · 수정 {rows.length - added - removed} · 지움 {removed})
        </span>
      </div>
      {rows.length > 0 && (
        <div className="table-scroll" style={{ maxHeight: "none" }}>
          <table className="table">
            <thead>
              <tr>
                <th>토큰</th>
                <th>언어</th>
                <th>변경</th>
                <th>상태</th>
              </tr>
            </thead>
            <tbody>
              {rows.slice(0, MAX_ROWS_SHOWN).map((r) => (
                <tr
                  key={`${r.token_id}|${r.locale}`}
                  className="clickable"
                  title="문구 표에서 이 셀 열기"
                  onClick={() => onOpenCell(r.token_id, r.locale)}
                >
                  <td>{r.token_key}</td>
                  <td>{r.locale}</td>
                  <td>
                    <Change old={r.old?.value} next={r.new?.value} />
                  </td>
                  <td className="muted">
                    {r.old?.status !== r.new?.status && `${r.old?.status ?? "∅"} → ${r.new?.status ?? "∅"}`}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {rows.length > MAX_ROWS_SHOWN && <div className="muted">앞 {MAX_ROWS_SHOWN}칸만 표시</div>}
    </div>
  );
}

// ── 파일 두 개 비교 ───────────────────────

type FileSide = { path: string; sheets: SheetPreview[]; mapping: ColumnMapping };

function FilesPanel({ run }: { run: RunFn }) {
  const [locales, setLocales] = useState<Locale[]>([]);
  const [a, setA] = useState<FileSide | null>(null);
  const [b, setB] = useState<FileSide | null>(null);
  const [rows, setRows] = useState<ValueDiff[] | null>(null);

  useEffect(() => {
    run(async () => setLocales(await api.listLocales()));
  }, [run]);
  useEffect(() => setRows(null), [a, b]);

  const pick = (set: (s: FileSide) => void) =>
    run(async () => {
      const p = await open({ filters: SHEET_FILTERS, multiple: false, directory: false });
      if (typeof p !== "string") return;
      const sheets = await api.workbookPreview(p);
      if (sheets[0]) set({ path: p, sheets, mapping: suggestMapping(sheets[0], 0, locales) });
    });

  const source = (s: FileSide | null): ImportSource | null =>
    s && mappingReady(s.mapping) ? { kind: "mapped", path: s.path, mapping: s.mapping } : null;
  const sa = source(a);
  const sb = source(b);

  const side = (label: string, s: FileSide | null, set: (s: FileSide) => void) => (
    <div className="card">
      <div className="row">
        <strong>{label}</strong>
        <button onClick={() => pick(set)}>파일 선택</button>
        <span className="path">{s?.path ?? ""}</span>
      </div>
      {s && (
        <MappingEditor sheets={s.sheets} locales={locales} mapping={s.mapping} onChange={(m) => set({ ...s, mapping: m })} />
      )}
    </div>
  );

  return (
    <div className="page">
      <div className="muted" style={{ marginBottom: 8 }}>
        두 엑셀 파일을 토큰명 × 언어 값으로 읽어 비교합니다(예: 지난 배포 엑셀과 이번 엑셀). 프로젝트 DB는 바뀌지 않습니다.
      </div>
      {side("이전 파일", a, setA)}
      {side("새 파일", b, setB)}
      <div className="row">
        <button className="primary" disabled={!sa || !sb} onClick={() => sa && sb && run(async () => setRows(await api.compareSources(sa, sb)))}>
          비교
        </button>
      </div>
      {rows && (
        <div className="card">
          <div className="stats">
            <span>
              바뀐 칸 <b>{rows.length}</b>
            </span>
          </div>
          {rows.length > 0 && (
            <div className="table-scroll" style={{ maxHeight: "none" }}>
              <table className="table">
                <thead>
                  <tr>
                    <th>토큰</th>
                    <th>언어</th>
                    <th>변경</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.slice(0, MAX_ROWS_SHOWN).map((r) => (
                    <tr key={`${r.key}|${r.locale}`}>
                      <td>{r.key}</td>
                      <td>{r.locale}</td>
                      <td>
                        <Change old={r.old} next={r.new} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
