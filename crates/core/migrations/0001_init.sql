-- 0001_init.sql
-- 마이그레이션은 추가만 한다. 이미 쓰인 파일은 고치지 말고 0002_*.sql 을 새로 만든다.
-- 모든 테이블은 STRICT. 연결마다 PRAGMA foreign_keys = ON (db.rs 에서 설정).

-- ── 언어 ─────────────────────────────────────────────
CREATE TABLE locale (
  code       TEXT PRIMARY KEY,                      -- BCP-47 (ko-KR, en-US ...)
  name       TEXT NOT NULL,
  is_source  INTEGER NOT NULL DEFAULT 0 CHECK (is_source IN (0, 1)),
  sort_order INTEGER NOT NULL DEFAULT 0
) STRICT;

-- ── 변경 묶음(= 버전) ─────────────────────────────────
CREATE TABLE changeset (
  id         INTEGER PRIMARY KEY,                   -- 전역 증가 번호
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  author     TEXT NOT NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('edit', 'import', 'revert', 'rename', 'admin')),
  message    TEXT,
  reverts_id INTEGER REFERENCES changeset(id)
) STRICT;

-- ── 토큰 ─────────────────────────────────────────────
-- id 는 바뀌지 않는 대리 키(UUIDv7). key(토큰명)는 바뀔 수 있다.
CREATE TABLE token (
  id          TEXT PRIMARY KEY,
  key         TEXT NOT NULL UNIQUE CHECK (key <> '' AND key = trim(key)),
  description TEXT,
  status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'deprecated')),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- 이전 토큰명. 다른 토큰이 재사용하지 못한다(아래 트리거).
CREATE TABLE token_key_history (
  old_key      TEXT PRIMARY KEY,
  token_id     TEXT NOT NULL REFERENCES token(id),
  changeset_id INTEGER NOT NULL REFERENCES changeset(id)
) STRICT;

CREATE TRIGGER token_key_no_reuse_ins BEFORE INSERT ON token
WHEN EXISTS (SELECT 1 FROM token_key_history WHERE old_key = NEW.key)
BEGIN SELECT RAISE(ABORT, 'token key was used by another token'); END;

CREATE TRIGGER token_key_no_reuse_upd BEFORE UPDATE OF key ON token
WHEN EXISTS (SELECT 1 FROM token_key_history WHERE old_key = NEW.key AND token_id <> NEW.id)
BEGIN SELECT RAISE(ABORT, 'token key was used by another token'); END;

CREATE TABLE token_tag (
  token_id TEXT NOT NULL REFERENCES token(id),
  tag      TEXT NOT NULL,
  PRIMARY KEY (token_id, tag)
) STRICT, WITHOUT ROWID;

-- ── 문구값(현재 값 캐시) ───────────────────────────────
-- 행이 없으면 미번역. 쓰기는 반드시 history::apply 를 거친다(아래 트리거가 강제).
-- outdated 판정: 원문 셀의 version > 이 셀의 version.
CREATE TABLE translation (
  token_id TEXT NOT NULL REFERENCES token(id),
  locale   TEXT NOT NULL REFERENCES locale(code),
  value    TEXT NOT NULL,
  status   TEXT NOT NULL DEFAULT 'translated' CHECK (status IN ('translated', 'reviewed', 'approved')),
  version  INTEGER NOT NULL REFERENCES changeset(id),
  PRIMARY KEY (token_id, locale)
) STRICT, WITHOUT ROWID;

-- ── 문구값 이력(추가만 함) ─────────────────────────────
CREATE TABLE translation_rev (
  changeset_id INTEGER NOT NULL REFERENCES changeset(id),
  token_id     TEXT NOT NULL,
  locale       TEXT NOT NULL,
  op           TEXT NOT NULL CHECK (op IN ('insert', 'update', 'delete')),
  old_value    TEXT,
  new_value    TEXT,
  old_status   TEXT,
  new_status   TEXT,
  PRIMARY KEY (token_id, locale, changeset_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX translation_rev_by_changeset ON translation_rev (changeset_id);

CREATE TRIGGER translation_needs_rev_ins BEFORE INSERT ON translation
WHEN NOT EXISTS (
  SELECT 1 FROM translation_rev r
  WHERE r.token_id = NEW.token_id AND r.locale = NEW.locale AND r.changeset_id = NEW.version
    AND r.new_value IS NEW.value AND r.new_status IS NEW.status)
BEGIN SELECT RAISE(ABORT, 'write translation_rev first (use history::apply)'); END;

CREATE TRIGGER translation_needs_rev_upd BEFORE UPDATE ON translation
WHEN NEW.version <= OLD.version
  OR NOT EXISTS (
    SELECT 1 FROM translation_rev r
    WHERE r.token_id = NEW.token_id AND r.locale = NEW.locale AND r.changeset_id = NEW.version
      AND r.new_value IS NEW.value AND r.new_status IS NEW.status)
BEGIN SELECT RAISE(ABORT, 'write translation_rev first (use history::apply)'); END;

CREATE TRIGGER translation_needs_rev_del BEFORE DELETE ON translation
WHEN NOT EXISTS (
  SELECT 1 FROM translation_rev r
  WHERE r.token_id = OLD.token_id AND r.locale = OLD.locale AND r.op = 'delete'
    AND r.changeset_id > OLD.version)
BEGIN SELECT RAISE(ABORT, 'write translation_rev first (use history::apply)'); END;

-- ── 그 밖의 엔티티 이력(토큰, 제약, 프로필, 매핑 ...) ─────
CREATE TABLE entity_rev (
  changeset_id INTEGER NOT NULL REFERENCES changeset(id),
  entity_type  TEXT NOT NULL,
  entity_id    TEXT NOT NULL,
  op           TEXT NOT NULL CHECK (op IN ('insert', 'update', 'delete')),
  old_json     TEXT CHECK (old_json IS NULL OR json_valid(old_json)),
  new_json     TEXT CHECK (new_json IS NULL OR json_valid(new_json)),
  PRIMARY KEY (entity_type, entity_id, changeset_id)
) STRICT, WITHOUT ROWID;

-- ── 릴리스 = 변경 묶음 번호에 붙인 이름표 ─────────────────
CREATE TABLE release (
  name         TEXT PRIMARY KEY,
  changeset_id INTEGER NOT NULL REFERENCES changeset(id),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  note         TEXT
) STRICT;

-- ── 제품 내보내기 ─────────────────────────────────────
CREATE TABLE export_target (
  id      TEXT PRIMARY KEY,
  name    TEXT NOT NULL,
  adapter TEXT NOT NULL DEFAULT 'table',            -- 포맷 어댑터 이름
  config  TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config))
) STRICT;

-- 토큰 ↔ 제품 키. 한 번 내보낸 키는 여기에 고정한다(규칙은 제안용).
CREATE TABLE token_external_key (
  target_id    TEXT NOT NULL REFERENCES export_target(id),
  external_key TEXT NOT NULL,
  token_id     TEXT NOT NULL REFERENCES token(id),
  PRIMARY KEY (target_id, external_key)
) STRICT, WITHOUT ROWID;

CREATE INDEX token_external_key_by_token ON token_external_key (token_id, target_id);

-- ── 측정 ─────────────────────────────────────────────
-- 폰트는 프로젝트 파일 안에 바이트로 담는다(파일 하나로 옮길 수 있게).
CREATE TABLE font_asset (
  id         TEXT PRIMARY KEY,
  family     TEXT NOT NULL,
  file_name  TEXT NOT NULL,
  sha256     TEXT NOT NULL UNIQUE,
  data       BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- 엔진 측정 DLL. id 는 tms_engine_version() 값.
CREATE TABLE measure_engine (
  id       TEXT PRIMARY KEY,
  dll_path TEXT NOT NULL,
  note     TEXT
) STRICT;

CREATE TABLE render_profile (
  id             TEXT PRIMARY KEY,
  name           TEXT NOT NULL UNIQUE,
  engine_id      TEXT NOT NULL REFERENCES measure_engine(id),
  size_px        REAL NOT NULL,
  letter_spacing REAL NOT NULL DEFAULT 0,
  line_height    REAL NOT NULL DEFAULT 0,               -- 0 = 엔진 기본값
  wrap_mode      INTEGER NOT NULL DEFAULT 1 CHECK (wrap_mode IN (0, 1)),
  -- 언어별 폰트 폴백 순서: {"*": ["font_id", ...], "ko-KR": ["font_id", ...]}
  fonts          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(fonts))
) STRICT;

-- ── 검증 제약 ─────────────────────────────────────────
-- 범위: global / tag / token. token + locale 이 가장 구체적이다.
CREATE TABLE constraint_def (
  id          TEXT PRIMARY KEY,
  validator   TEXT NOT NULL,                            -- text.length, render.max_width ...
  params      TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(params)),
  scope_kind  TEXT NOT NULL CHECK (scope_kind IN ('global', 'tag', 'token')),
  scope_ref   TEXT,                                     -- tag 이름 또는 token id
  locale      TEXT REFERENCES locale(code),             -- NULL = 전 언어
  severity    TEXT NOT NULL DEFAULT 'error' CHECK (severity IN ('error', 'warning', 'info')),
  enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  description TEXT,
  CHECK ((scope_kind = 'global') = (scope_ref IS NULL))
) STRICT;
