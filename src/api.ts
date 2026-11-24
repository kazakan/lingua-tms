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
  tags: string[];
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
  unresolved_keys: string[];
  unchanged: number;
  rows: number;
}

export interface ImportResult {
  changeset_id: number | null;
  created_tokens: number;
  changed_cells: number;
}

// ── 이력 ─────────────────────────────────

export interface ChangesetInfo {
  id: number;
  created_at: string;
  author: string;
  kind: "edit" | "import" | "revert" | "rename" | "admin";
  message: string | null;
  reverts_id: number | null;
  cell_changes: number;
  entity_changes: number;
  releases: string[];
}

export interface Release {
  name: string;
  changeset_id: number;
  created_at: string;
  note: string | null;
}

export interface KeyedDiff {
  token_id: string;
  token_key: string;
  locale: string;
  old: CellValue | null;
  new: CellValue | null;
}

export interface RevertOutcome {
  changeset_id: number | null;
  /** 대상 변경 묶음 뒤에 다시 바뀌어 건너뛴 칸 */
  conflicts: { token_id: string; locale: string }[];
}

export interface ValueDiff {
  key: string;
  locale: string;
  old: string | null;
  new: string | null;
}

// ── 내보내기 ─────────────────────────────

export interface AdapterInfo {
  id: string;
  description: string;
}

export interface ExportTarget {
  id: string;
  name: string;
  adapter: string;
  config: unknown;
}

export interface ExportTargetInput {
  id: string | null;
  name: string;
  adapter: string;
  config: unknown;
}

/** 설정형 표 어댑터("table")의 설정 */
export interface TableConfig {
  sheet_name: string;
  key_header: string;
  columns: { header: string; locale: string }[];
  key_rule: { prefix: string; case: "keep" | "upper" | "lower"; dot_replacement: string | null };
  fallback_to_source: boolean;
  include_deprecated: boolean;
}

export interface ExportRow {
  token_id: string;
  token_key: string;
  external_key: string;
  /** 이미 고정된 키면 true, 이번에 처음 정해지는 제안 키면 false */
  fixed: boolean;
  values: Record<string, string>;
}

export interface ExportPreview {
  target: ExportTarget;
  locales: string[];
  rows: ExportRow[];
  new_keys: number;
}

export interface ExportReport {
  rows: number;
  new_keys: number;
  changeset_id: number | null;
}

export interface ExportIssue {
  token_key: string;
  locale: string;
  validator: string;
  message: string;
}

/** error 는 내보내기를 막고, warning 은 사유를 적어야 진행한다 */
export interface ExportCheck {
  errors: ExportIssue[];
  warnings: ExportIssue[];
}

// ── 검증 ─────────────────────────────────

export interface ValidatorInfo {
  id: string;
  description: string;
  params_schema: unknown;
  needs_render: boolean;
}

export type Severity = "error" | "warning" | "info";

/** 전역 / 태그 / 토큰(id) */
export type Scope = { kind: "global" } | { kind: "tag"; ref: string } | { kind: "token"; ref: string };

export interface ConstraintDef {
  id: string;
  validator: string;
  params: Record<string, unknown>;
  scope: Scope;
  /** null = 전 언어 */
  locale: string | null;
  severity: Severity;
  enabled: boolean;
  description: string | null;
}

export interface ConstraintInput extends Omit<ConstraintDef, "id"> {
  id: string | null;
}

export interface Outcome {
  verdict: "pass" | "fail";
  message: string | null;
  metrics: unknown;
}

export interface Finding {
  constraint_id: string;
  validator: string;
  severity: Severity;
  outcome: Outcome | null;
  /** 검증기 자체 오류(파라미터 오류 등) */
  error: string | null;
}

export interface CellFindings {
  token_id: string;
  token_key: string;
  locale: string;
  findings: Finding[];
}

export interface ValidationReport {
  cells: CellFindings[];
  summary: {
    cells: number;
    checks: number;
    error: number;
    warning: number;
    info: number;
    broken: number;
    skipped_render: number;
  };
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
  listChangesets: (limit: number) => invoke<ChangesetInfo[]>("list_changesets", { limit }),
  listReleases: () => invoke<Release[]>("list_releases"),
  createRelease: (name: string, changesetId: number | null, note: string | null) =>
    invoke<number>("create_release", { name, changesetId, note }),
  /** 0 = 처음(빈 상태) */
  diffChangesets: (from: number, to: number) => invoke<KeyedDiff[]>("diff_changesets", { from, to }),
  revertChangeset: (id: number) => invoke<RevertOutcome>("revert_changeset", { id }),
  rollbackTo: (changesetId: number) => invoke<number | null>("rollback_to", { changesetId }),
  compareSources: (a: ImportSource, b: ImportSource) => invoke<ValueDiff[]>("compare_sources", { a, b }),
  listAdapters: () => invoke<AdapterInfo[]>("list_adapters"),
  listExportTargets: () => invoke<ExportTarget[]>("list_export_targets"),
  saveExportTarget: (target: ExportTargetInput) => invoke<string>("save_export_target", { target }),
  deleteExportTarget: (id: string) => invoke<void>("delete_export_target", { id }),
  planExport: (targetId: string) => invoke<ExportPreview>("plan_export", { targetId }),
  exportToFile: (targetId: string, path: string, note: string | null) =>
    invoke<ExportReport>("export_to_file", { targetId, path, note }),
  fixExternalKey: (targetId: string, tokenId: string, key: string) =>
    invoke<number>("fix_external_key", { targetId, tokenId, key }),
  checkExport: (targetId: string) => invoke<ExportCheck>("check_export", { targetId }),
  listValidators: () => invoke<ValidatorInfo[]>("list_validators"),
  listConstraints: () => invoke<ConstraintDef[]>("list_constraints"),
  saveConstraint: (input: ConstraintInput) => invoke<string>("save_constraint", { input }),
  deleteConstraint: (id: string) => invoke<void>("delete_constraint", { id }),
  validateAll: () => invoke<ValidationReport>("validate_all"),
  listTags: () => invoke<string[]>("list_tags"),
  setTokenTags: (tokenId: string, tags: string[]) => invoke<number | null>("set_token_tags", { tokenId, tags }),
};
