-- Этап 7: авторитетность комментариев и устаревание зависимых результатов.
--
-- Официальным считается только ответ преподавателя или ассистента, который
-- пользователь явно подтвердил. Требования выводятся только из официальных
-- ответов. Проверки, промты, готовность и комплекты запоминают отпечаток
-- входных данных (workflow/fingerprint.rs); при несовпадении с текущим они
-- устарели, но остаются в истории.

ALTER TABLE teacher_comments ADD COLUMN authority TEXT NOT NULL DEFAULT 'unknown'
    CHECK (authority IN ('teacher_official', 'teaching_assistant', 'student',
                         'group_chat_unverified', 'user_note', 'system', 'unknown'));
ALTER TABLE teacher_comments ADD COLUMN author TEXT NOT NULL DEFAULT '';
ALTER TABLE teacher_comments ADD COLUMN context TEXT NOT NULL DEFAULT '';
ALTER TABLE teacher_comments ADD COLUMN official_confirmed_at TEXT;

ALTER TABLE teacher_requirements ADD COLUMN withdrawn_at TEXT;

-- NULL у записей до этапа 7: они не сопоставлены с входными данными и считаются устаревшими.
ALTER TABLE review_runs ADD COLUMN inputs_fingerprint TEXT;
ALTER TABLE revision_prompts ADD COLUMN inputs_fingerprint TEXT;
ALTER TABLE submission_packages ADD COLUMN inputs_fingerprint TEXT;
ALTER TABLE homeworks ADD COLUMN ready_fingerprint TEXT;

-- Журнал решений об авторитетности (только добавление).
CREATE TABLE comment_authority_events (
    id TEXT PRIMARY KEY NOT NULL,
    comment_id TEXT NOT NULL REFERENCES teacher_comments(id) ON DELETE CASCADE,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    authority TEXT NOT NULL,
    author TEXT NOT NULL,
    official INTEGER NOT NULL CHECK (official IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX comment_authority_events_comment ON comment_authority_events(comment_id, created_at);

CREATE TRIGGER comment_authority_events_no_update
BEFORE UPDATE ON comment_authority_events
BEGIN
    SELECT RAISE(ABORT, 'comment authority events are append-only');
END;

CREATE TRIGGER teacher_comments_official_authority_insert
BEFORE INSERT ON teacher_comments
WHEN NEW.official_confirmed_at IS NOT NULL
 AND NEW.authority NOT IN ('teacher_official', 'teaching_assistant')
BEGIN
    SELECT RAISE(ABORT, 'only a teacher or teaching assistant answer can be official');
END;

CREATE TRIGGER teacher_comments_official_authority_update
BEFORE UPDATE OF authority, official_confirmed_at ON teacher_comments
WHEN NEW.official_confirmed_at IS NOT NULL
 AND NEW.authority NOT IN ('teacher_official', 'teaching_assistant')
BEGIN
    SELECT RAISE(ABORT, 'only a teacher or teaching assistant answer can be official');
END;

CREATE TRIGGER teacher_requirements_official_source
BEFORE INSERT ON teacher_requirements
WHEN NEW.comment_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM teacher_comments
     WHERE id = NEW.comment_id AND official_confirmed_at IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'a requirement can only come from an official answer');
END;

CREATE TRIGGER teacher_requirements_withdrawn_final
BEFORE UPDATE OF text, origin, status, note, checked_version_id, withdrawn_at ON teacher_requirements
WHEN OLD.withdrawn_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a withdrawn requirement is kept for history and cannot change');
END;
