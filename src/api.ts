// Rust 명령(src-tauri/src/lib.rs)의 타입 있는 래퍼. 명령을 추가하면 여기도 같이 고친다.
import { invoke } from "@tauri-apps/api/core";

export interface ProjectInfo {
  path: string;
  schema_version: number;
}

export interface Token {
  id: string;
  key: string;
  description: string | null;
  status: string;
}

export interface Locale {
  code: string;
  name: string;
  is_source: boolean;
  sort_order: number;
}

export interface GridCell {
  value: string;
  status: string;
  version: number;
  /** 원문이 이 셀보다 나중에 바뀜 */
  outdated: boolean;
}

export interface GridRow {
  token: Token;
  cells: Record<string, GridCell>;
}

export interface Grid {
  locales: Locale[];
  source_locale: string | null;
  rows: GridRow[];
}

export interface CellValue {
  value: string;
  status: string;
}

export interface RevRow {
  changeset_id: number;
  op: "insert" | "update" | "delete";
  old: CellValue | null;
  new: CellValue | null;
  author: string;
  created_at: string;
}

export interface ValidatorInfo {
  id: string;
  description: string;
  params_schema: unknown;
  needs_render: boolean;
}

export const api = {
  openProject: (path: string) => invoke<ProjectInfo>("open_project", { path }),
  projectInfo: () => invoke<ProjectInfo | null>("project_info"),
  loadGrid: () => invoke<Grid>("load_grid"),
  addLocale: (code: string, name: string, isSource: boolean) =>
    invoke<void>("add_locale", { code, name, isSource }),
  createToken: (key: string, description?: string) =>
    invoke<string>("create_token", { key, description: description ?? null }),
  renameToken: (tokenId: string, newKey: string) =>
    invoke<number | null>("rename_token", { tokenId, newKey }),
  /** 빈 문자열이면 미번역으로 되돌린다. 바뀐 게 없으면 null. */
  setCell: (tokenId: string, locale: string, value: string) =>
    invoke<number | null>("set_cell", { tokenId, locale, value }),
  cellHistory: (tokenId: string, locale: string) =>
    invoke<RevRow[]>("cell_history", { tokenId, locale }),
  listValidators: () => invoke<ValidatorInfo[]>("list_validators"),
};
