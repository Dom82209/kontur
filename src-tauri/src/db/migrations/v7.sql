-- Схема v7 (этап 9): задания передачи материалов провайдерам AI.
-- v1–v6 не изменяются. Единственная новая таблица — ai_jobs: аналога в
-- старой схеме нет. Задание хранит ровно тот текст, который показан
-- пользователю перед отправкой, ответ провайдера, черновик разбора и журнал
-- без секретов. Учётные данные провайдеров не хранятся нигде.

CREATE TABLE ai_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
    task TEXT NOT NULL CHECK (task IN (
        'material_analysis', 'initial_prompt', 'solution_review', 'revision_prompt', 'teacher_review'
    )),
    provider TEXT NOT NULL CHECK (provider IN ('codex', 'claude')),
    transport TEXT NOT NULL CHECK (transport IN ('cli', 'manual_handoff')),
    adapter_label TEXT CHECK (adapter_label IS NULL OR length(adapter_label) <= 120),
    status TEXT NOT NULL CHECK (status IN (
        'prepared', 'running', 'response_ready', 'failed', 'cancelled', 'confirmed', 'discarded'
    )),
    include_prompt INTEGER NOT NULL DEFAULT 0 CHECK (include_prompt IN (0, 1)),
    prompt_text TEXT NOT NULL,
    prompt_sha256 TEXT NOT NULL CHECK (length(prompt_sha256) = 64 AND prompt_sha256 NOT GLOB '*[^0-9a-f]*'),
    prompt_chars INTEGER NOT NULL CHECK (prompt_chars >= 0),
    context_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(context_json)),
    context_truncated INTEGER NOT NULL DEFAULT 0 CHECK (context_truncated IN (0, 1)),
    limit_chars INTEGER NOT NULL CHECK (limit_chars > 0),
    response_text TEXT,
    response_source TEXT CHECK (response_source IS NULL OR response_source IN ('cli', 'paste', 'file')),
    response_file_name TEXT CHECK (
        response_file_name IS NULL OR (
            length(response_file_name) BETWEEN 1 AND 255
            AND instr(response_file_name, '/') = 0 AND instr(response_file_name, '\') = 0
        )
    ),
    response_sha256 TEXT CHECK (response_sha256 IS NULL OR length(response_sha256) = 64),
    response_truncated INTEGER NOT NULL DEFAULT 0 CHECK (response_truncated IN (0, 1)),
    parse_status TEXT CHECK (parse_status IS NULL OR parse_status IN ('valid', 'partial', 'corrupt', 'empty')),
    draft_json TEXT CHECK (draft_json IS NULL OR json_valid(draft_json)),
    issues_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(issues_json)),
    log_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(log_json)),
    error_code TEXT,
    error_message TEXT CHECK (error_message IS NULL OR length(error_message) <= 2000),
    exit_code INTEGER,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    sent_at TEXT,
    finished_at TEXT,
    confirmed_at TEXT,
    edited INTEGER CHECK (edited IS NULL OR edited IN (0, 1)),
    prompt_id TEXT REFERENCES revision_prompts(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((status = 'confirmed') = (confirmed_at IS NOT NULL)),
    CHECK (status <> 'response_ready' OR response_text IS NOT NULL),
    CHECK (transport = 'cli' OR response_source IS NULL OR response_source <> 'cli')
) STRICT;

CREATE INDEX idx_ai_jobs_homework ON ai_jobs(homework_id, created_at);
CREATE INDEX idx_ai_jobs_run ON ai_jobs(run_id, status);
-- Одновременно у запуска может выполняться только одно задание.
CREATE UNIQUE INDEX idx_ai_jobs_one_running ON ai_jobs(run_id) WHERE status = 'running';

-- Переданный текст, его состав и привязки неизменяемы.
CREATE TRIGGER ai_jobs_payload_immutable
BEFORE UPDATE OF id, homework_id, run_id, task, provider, transport, include_prompt, prompt_text,
    prompt_sha256, prompt_chars, context_json, context_truncated, limit_chars, created_at ON ai_jobs
WHEN NEW.id IS NOT OLD.id OR NEW.homework_id IS NOT OLD.homework_id OR NEW.run_id IS NOT OLD.run_id
  OR NEW.task IS NOT OLD.task OR NEW.provider IS NOT OLD.provider OR NEW.transport IS NOT OLD.transport
  OR NEW.include_prompt IS NOT OLD.include_prompt OR NEW.prompt_text IS NOT OLD.prompt_text
  OR NEW.prompt_sha256 IS NOT OLD.prompt_sha256 OR NEW.prompt_chars IS NOT OLD.prompt_chars
  OR NEW.context_json IS NOT OLD.context_json OR NEW.context_truncated IS NOT OLD.context_truncated
  OR NEW.limit_chars IS NOT OLD.limit_chars OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'ai job payload is immutable');
END;

-- Завершённое задание неизменяемо (подтверждение, отказ, отмена, ошибка).
CREATE TRIGGER ai_jobs_terminal_immutable
BEFORE UPDATE ON ai_jobs
WHEN OLD.status IN ('confirmed', 'discarded', 'cancelled', 'failed')
BEGIN
    SELECT RAISE(ABORT, 'finished ai job is immutable');
END;

-- Журнал, состав и замечания не содержат абсолютных путей и выхода за корень.
CREATE TRIGGER ai_jobs_no_paths_insert
BEFORE INSERT ON ai_jobs
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.context_json) WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%' OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
    )
) OR EXISTS (
    SELECT 1 FROM json_tree(NEW.log_json) WHERE type = 'text' AND (
        value LIKE '%/Users/%' OR value LIKE '%/home/%' OR lower(value) LIKE '%file:%'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'ai job metadata must not contain absolute paths');
END;

CREATE TRIGGER ai_jobs_no_paths_update
BEFORE UPDATE OF log_json, issues_json, error_message ON ai_jobs
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.log_json) WHERE type = 'text' AND (
        value LIKE '%/Users/%' OR value LIKE '%/home/%' OR lower(value) LIKE '%file:%'
    )
) OR EXISTS (
    SELECT 1 FROM json_tree(NEW.issues_json) WHERE type = 'text' AND (
        value LIKE '%/Users/%' OR value LIKE '%/home/%'
    )
) OR coalesce(NEW.error_message, '') LIKE '%/Users/%' OR coalesce(NEW.error_message, '') LIKE '%/home/%'
BEGIN
    SELECT RAISE(ABORT, 'ai job log must not contain absolute paths');
END;

-- Отправленная работа неизменяема: новых заданий у неё нет.
CREATE TRIGGER submitted_homeworks_no_new_ai_job
BEFORE INSERT ON ai_jobs
WHEN EXISTS (SELECT 1 FROM homeworks h WHERE h.id = NEW.homework_id AND (h.stage = 'submitted' OR h.workflow_stage = 'submitted'))
BEGIN
    SELECT RAISE(ABORT, 'submitted homework is immutable');
END;

-- Задание относится к запуску того же ДЗ.
CREATE TRIGGER ai_jobs_run_of_homework
BEFORE INSERT ON ai_jobs
WHEN NOT EXISTS (SELECT 1 FROM review_runs r WHERE r.id = NEW.run_id AND r.homework_id = NEW.homework_id)
BEGIN
    SELECT RAISE(ABORT, 'ai job must reference a run of its homework');
END;
