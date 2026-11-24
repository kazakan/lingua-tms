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

/** 중첩 객체의 필드는 Tauri가 변환하지 않으므로 Rust와 같은 snake_case로 쓴다. */
export interface CellEdit {
  token_id: string;
  locale: string;
  value: string;
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

// ── 가져오기 ─────────────────────────────

export interface SheetPreview {
  name: string;
  rows: number;
  cols: number;
  /** 앞쪽 30행. 열 번호는 엑셀과 같다(A = 0). */
  head: string[][];
}

export interface LocaleColumn {
  col: number;
  locale: string;
}

export interface ColumnMapping {
  sheet: string;
  /** 0부터. 데이터는 그다음 행부터 */
  header_row: number;
  key_col: number;
  locale_cols: LocaleColumn[];
}

export type ImportSource =
  | { kind: "mapped"; path: string; mapping: ColumnMapping }
  | { kind: "target"; path: string; target_id: string };

export interface ImportOptions {
  create_missing_tokens: boolean;
  clear_empty: boolean;
}

export interface PlannedChange {
  token_id: string | null;
  token_key: string;
  locale: string;
  old: string | null;
  new: string | null;
}

export interface ImportPlan {
  changes: PlannedChange[];
  new_tokens: string[];
  renamed: { sheet_key: string; current_key: string }[];
  skipped_tokens: string[];
  conflicting_keys: string[];
  unknown_locales: string[];
  duplicate_keys: string[];
  unchanged: number;
  rows: number;
}

export interface ImportResult {
  changeset_id: number | null;
  created_tokens: number;
  changed_cells: number;
}

// ── 검증 ─────────────────────────────────

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
  listLocales: () => invoke<Locale[]>("list_locales"),
  listTokens: () => invoke<Token[]>("list_tokens"),
  createToken: (key: string, description?: string) =>
    invoke<string>("create_token", { key, description: description ?? null }),
  renameToken: (tokenId: string, newKey: string) =>
    invoke<number | null>("rename_token", { tokenId, newKey }),
  /** 여러 셀을 변경 묶음 하나로 쓴다. 빈 문자열은 미번역, 값이 같은 셀은 건너뜀. 바뀐 게 없으면 null. */
  setCells: (edits: CellEdit[]) => invoke<number | null>("set_cells", { edits }),
  cellHistory: (tokenId: string, locale: string) =>
    invoke<RevRow[]>("cell_history", { tokenId, locale }),
  workbookPreview: (path: string) => invoke<SheetPreview[]>("workbook_preview", { path }),
  planImport: (source: ImportSource, options: ImportOptions) =>
    invoke<ImportPlan>("plan_import", { source, options }),
  applyImport: (source: ImportSource, options: ImportOptions) =>
    invoke<ImportResult>("apply_import", { source, options }),
  listValidators: () => invoke<ValidatorInfo[]>("list_validators"),
};
