use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Project, Task, TaskStatus, normalize_project_name},
    sync_protocol::{
        SyncEntityKind, SyncProject, SyncRequest, SyncResponse, SyncTask, SyncTombstone,
    },
};

const SCHEMA_VERSION: i64 = 3;
const INBOX: &str = "inbox";

pub struct Storage {
    conn: Connection,
    db_path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct VisibleIdMapping {
    version: u8,
    task_uuids: Vec<String>,
}

const VISIBLE_ID_MAPPING_VERSION: u8 = 1;

impl Storage {
    pub fn open_default() -> Result<Self, AppError> {
        Self::open(database_path()?)
    }

    pub fn open(path: PathBuf) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&path)?;
        let storage = Self {
            conn,
            db_path: path,
        };
        storage.initialize()?;
        Ok(storage)
    }

    pub fn initialize(&self) -> Result<(), AppError> {
        self.conn.pragma_update(None, "foreign_keys", "ON")?;

        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        match version {
            0 => self.create_schema()?,
            SCHEMA_VERSION => {}
            version => {
                return Err(AppError::StorageMessage(format!(
                    "unsupported database schema version {version}"
                )));
            }
        }

        self.ensure_project(INBOX)?;
        Ok(())
    }

    fn create_schema(&self) -> Result<(), AppError> {
        self.conn.execute_batch(&format!(
            "
            CREATE TABLE projects (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              uuid TEXT NOT NULL UNIQUE,
              name TEXT NOT NULL,
              name_normalized TEXT NOT NULL UNIQUE,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              archived_at TEXT,
              sync_revision INTEGER NOT NULL DEFAULT 0,
              sync_dirty INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE tasks (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              uuid TEXT NOT NULL UNIQUE,
              project_id INTEGER NOT NULL REFERENCES projects(id),
              sort_order INTEGER NOT NULL,
              title TEXT NOT NULL,
              notes TEXT NOT NULL DEFAULT '',
              status TEXT NOT NULL CHECK (status IN ('pending', 'done')),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              completed_at TEXT,
              sync_revision INTEGER NOT NULL DEFAULT 0,
              sync_dirty INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE sync_tombstones (
              entity_type TEXT NOT NULL CHECK (entity_type IN ('project', 'task')),
              uuid TEXT NOT NULL,
              deleted_at TEXT NOT NULL,
              sync_revision INTEGER NOT NULL DEFAULT 0,
              sync_dirty INTEGER NOT NULL DEFAULT 1,
              PRIMARY KEY (entity_type, uuid)
            );

            CREATE TABLE sync_state (
              key TEXT PRIMARY KEY,
              value INTEGER NOT NULL
            );

            INSERT INTO sync_state (key, value) VALUES ('checkpoint', 0);

            PRAGMA user_version = {SCHEMA_VERSION};
            "
        ))?;
        Ok(())
    }

    pub fn ensure_project(&self, name: &str) -> Result<Project, AppError> {
        let normalized = normalize_project_name(name);
        if normalized.is_empty() {
            return Err(AppError::User("Project name cannot be empty.".to_string()));
        }

        if let Some(project) = self.find_project(name)? {
            return Ok(project);
        }

        let now = now_timestamp();
        self.conn.execute(
            "
            INSERT INTO projects (uuid, name, name_normalized, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?4)
            ",
            params![Uuid::new_v4().to_string(), name.trim(), normalized, now],
        )?;

        let id = self.conn.last_insert_rowid();
        self.get_project_by_id(id)?.ok_or_else(|| {
            AppError::StorageMessage("created project could not be loaded".to_string())
        })
    }

    pub fn find_project(&self, name: &str) -> Result<Option<Project>, AppError> {
        let normalized = normalize_project_name(name);
        self.conn
            .query_row(
                "
                SELECT id, uuid, name, name_normalized, created_at, updated_at, archived_at
                FROM projects
                WHERE name_normalized = ?1
                ",
                params![normalized],
                project_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    pub fn get_project_by_id(&self, id: i64) -> Result<Option<Project>, AppError> {
        self.conn
            .query_row(
                "
                SELECT id, uuid, name, name_normalized, created_at, updated_at, archived_at
                FROM projects
                WHERE id = ?1
                ",
                params![id],
                project_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    pub fn add_task(&self, project_name: &str, title: &str) -> Result<Task, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::User("Task title cannot be empty.".to_string()));
        }

        let project = self.ensure_project(project_name)?;
        let sort_order = self.next_pending_sort_order(project.id)?;
        let now = now_timestamp();
        self.conn.execute(
            "
            INSERT INTO tasks (uuid, project_id, sort_order, title, notes, status, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?6)
            ",
            params![
                Uuid::new_v4().to_string(),
                project.id,
                sort_order,
                title,
                TaskStatus::Pending.as_str(),
                now
            ],
        )?;

        let id = self.conn.last_insert_rowid();
        self.get_task(id)?
            .ok_or_else(|| AppError::StorageMessage("created task could not be loaded".to_string()))
    }

    pub fn get_task(&self, id: i64) -> Result<Option<Task>, AppError> {
        self.conn
            .query_row(
                "
                SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                       t.status, t.created_at, t.updated_at, t.completed_at
                FROM tasks t
                JOIN projects p ON p.id = t.project_id
                WHERE t.id = ?1
                ",
                params![id],
                task_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    pub fn get_task_by_uuid(&self, uuid: &str) -> Result<Option<Task>, AppError> {
        self.conn
            .query_row(
                "
                SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                       t.status, t.created_at, t.updated_at, t.completed_at
                FROM tasks t
                JOIN projects p ON p.id = t.project_id
                WHERE t.uuid = ?1
                ",
                params![uuid],
                task_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    pub fn save_visible_id_mapping(&self, tasks: &[Task]) -> Result<(), AppError> {
        let mapping = VisibleIdMapping {
            version: VISIBLE_ID_MAPPING_VERSION,
            task_uuids: tasks.iter().map(|task| task.uuid.clone()).collect(),
        };
        let contents = serde_json::to_string_pretty(&mapping)
            .map_err(|error| AppError::StorageMessage(error.to_string()))?;
        fs::write(self.visible_id_mapping_path(), contents)?;
        Ok(())
    }

    pub fn load_visible_id_mapping(&self) -> Result<Option<Vec<String>>, AppError> {
        let path = self.visible_id_mapping_path();
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AppError::from(error)),
        };
        let mapping: VisibleIdMapping = serde_json::from_str(&contents).map_err(|_| {
            AppError::User("Could not read the last task list; run ct list again.".to_string())
        })?;
        if mapping.version != VISIBLE_ID_MAPPING_VERSION {
            return Err(AppError::User(
                "Could not read the last task list; run ct list again.".to_string(),
            ));
        }
        Ok(Some(mapping.task_uuids))
    }

    pub fn list_pending_tasks(&self) -> Result<Vec<Task>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                   t.status, t.created_at, t.updated_at, t.completed_at
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE t.status = 'pending'
            ORDER BY CASE WHEN p.name_normalized = 'inbox' THEN 0 ELSE 1 END,
                     p.name COLLATE NOCASE,
                     t.sort_order,
                     t.id
            ",
        )?;

        let tasks = statement
            .query_map([], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tasks)
    }

    pub fn list_pending_tasks_for_project(
        &self,
        name: &str,
    ) -> Result<Option<(Project, Vec<Task>)>, AppError> {
        let Some(project) = self.find_project(name)? else {
            return Ok(None);
        };

        let mut statement = self.conn.prepare(
            "
            SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                   t.status, t.created_at, t.updated_at, t.completed_at
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE t.status = 'pending'
              AND p.id = ?1
            ORDER BY t.sort_order, t.id
            ",
        )?;

        let tasks = statement
            .query_map(params![project.id], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((project, tasks)))
    }

    pub fn list_completed_tasks(&self) -> Result<Vec<Task>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                   t.status, t.created_at, t.updated_at, t.completed_at
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE t.status = 'done'
            ORDER BY CASE WHEN p.name_normalized = 'inbox' THEN 0 ELSE 1 END,
                     p.name COLLATE NOCASE,
                     t.completed_at DESC,
                     t.id DESC
            ",
        )?;

        let tasks = statement
            .query_map([], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tasks)
    }

    pub fn list_completed_tasks_for_project(
        &self,
        name: &str,
    ) -> Result<Option<(Project, Vec<Task>)>, AppError> {
        let Some(project) = self.find_project(name)? else {
            return Ok(None);
        };

        let mut statement = self.conn.prepare(
            "
            SELECT t.id, t.uuid, t.project_id, p.name, t.sort_order, t.title, t.notes,
                   t.status, t.created_at, t.updated_at, t.completed_at
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE t.status = 'done'
              AND p.id = ?1
            ORDER BY t.completed_at DESC, t.id DESC
            ",
        )?;

        let tasks = statement
            .query_map(params![project.id], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((project, tasks)))
    }

    pub fn mark_done(&self, id: i64) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let now = now_timestamp();
        self.conn.execute(
            "
            UPDATE tasks
            SET status = 'done', completed_at = ?1, updated_at = ?1, sync_dirty = 1
            WHERE id = ?2
            ",
            params![now, id],
        )?;
        self.compact_pending_sort_orders(task.project_id)?;

        Ok(Some(task))
    }

    pub fn mark_undone(&self, id: i64) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let sort_order = if task.status == TaskStatus::Done {
            self.compact_pending_sort_orders(task.project_id)?;
            self.next_pending_sort_order(task.project_id)?
        } else {
            task.sort_order
        };
        let now = now_timestamp();
        self.conn.execute(
            "
            UPDATE tasks
            SET status = 'pending', completed_at = NULL, sort_order = ?1, updated_at = ?2, sync_dirty = 1
            WHERE id = ?3
            ",
            params![sort_order, now, id],
        )?;

        Ok(Some(task))
    }

    pub fn delete_task(&self, id: i64) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let now = now_timestamp();
        self.conn.execute(
            "
            INSERT INTO sync_tombstones (entity_type, uuid, deleted_at, sync_revision, sync_dirty)
            VALUES ('task', ?1, ?2, 0, 1)
            ON CONFLICT(entity_type, uuid) DO UPDATE SET
              deleted_at = excluded.deleted_at,
              sync_revision = 0,
              sync_dirty = 1
            ",
            params![task.uuid, now],
        )?;
        self.conn.execute(
            "
            DELETE FROM tasks
            WHERE id = ?1
            ",
            params![id],
        )?;
        if task.status == TaskStatus::Pending {
            self.compact_pending_sort_orders(task.project_id)?;
        }

        Ok(Some(task))
    }

    pub fn move_task(&self, id: i64, project_name: &str) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let project = self.ensure_project(project_name)?;
        let sort_order = self.next_pending_sort_order(project.id)?;
        let now = now_timestamp();
        self.conn.execute(
            "
            UPDATE tasks
            SET project_id = ?1, sort_order = ?2, updated_at = ?3, sync_dirty = 1
            WHERE id = ?4
            ",
            params![project.id, sort_order, now, id],
        )?;
        if task.status == TaskStatus::Pending && task.project_id != project.id {
            self.compact_pending_sort_orders(task.project_id)?;
        }

        self.get_task(id)
    }

    pub fn append_note(&self, id: i64, note: &str) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let notes = if task.notes.is_empty() {
            note.to_string()
        } else {
            format!("{}\n{}", task.notes, note)
        };
        let now = now_timestamp();
        self.conn.execute(
            "
            UPDATE tasks
            SET notes = ?1, updated_at = ?2, sync_dirty = 1
            WHERE id = ?3
            ",
            params![notes, now, id],
        )?;

        self.get_task(id)
    }

    pub fn update_task_details(
        &self,
        id: i64,
        project_name: &str,
        title: &str,
        notes: &str,
    ) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::User("Task title cannot be empty.".to_string()));
        }

        let project = self.ensure_project(project_name)?;
        let sort_order = if project.id == task.project_id {
            task.sort_order
        } else {
            self.next_pending_sort_order(project.id)?
        };
        let now = now_timestamp();
        self.conn.execute(
            "
            UPDATE tasks
            SET project_id = ?1, sort_order = ?2, title = ?3, notes = ?4, updated_at = ?5, sync_dirty = 1
            WHERE id = ?6
            ",
            params![project.id, sort_order, title, notes, now, id],
        )?;
        if task.status == TaskStatus::Pending && task.project_id != project.id {
            self.compact_pending_sort_orders(task.project_id)?;
        }

        self.get_task(id)
    }

    pub fn clear_completed_tasks(&self, project_name: Option<&str>) -> Result<usize, AppError> {
        match project_name {
            Some(project_name) => {
                let project = self
                    .find_project(project_name)?
                    .ok_or_else(|| AppError::User(format!("Project not found: {project_name}")))?;
                let tasks = self.completed_task_tombstone_rows(Some(project.id))?;
                for (uuid, deleted_at) in tasks {
                    self.record_tombstone(SyncEntityKind::Task, &uuid, &deleted_at)?;
                }
                self.conn
                    .execute(
                        "
                        DELETE FROM tasks
                        WHERE status = 'done'
                          AND project_id = ?1
                        ",
                        params![project.id],
                    )
                    .map_err(AppError::from)
            }
            None => {
                let tasks = self.completed_task_tombstone_rows(None)?;
                for (uuid, deleted_at) in tasks {
                    self.record_tombstone(SyncEntityKind::Task, &uuid, &deleted_at)?;
                }
                self.conn
                    .execute(
                        "
                        DELETE FROM tasks
                        WHERE status = 'done'
                        ",
                        [],
                    )
                    .map_err(AppError::from)
            }
        }
    }

    fn next_pending_sort_order(&self, project_id: i64) -> Result<i64, AppError> {
        self.conn
            .query_row(
                "
                SELECT COALESCE(MAX(sort_order), 0) + 1
                FROM tasks
                WHERE project_id = ?1
                  AND status = 'pending'
                ",
                params![project_id],
                |row| row.get(0),
            )
            .map_err(AppError::from)
    }

    fn compact_pending_sort_orders(&self, project_id: i64) -> Result<(), AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT id
            FROM tasks
            WHERE project_id = ?1
              AND status = 'pending'
            ORDER BY sort_order, id
            ",
        )?;
        let ids = statement
            .query_map(params![project_id], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;

        let now = now_timestamp();
        for (index, task_id) in ids.iter().enumerate() {
            let sort_order = i64::try_from(index + 1).unwrap_or(i64::MAX);
            self.conn.execute(
                "
                UPDATE tasks
                SET sort_order = ?1,
                    updated_at = CASE WHEN sort_order != ?1 THEN ?3 ELSE updated_at END,
                    sync_dirty = CASE WHEN sort_order != ?1 THEN 1 ELSE sync_dirty END
                WHERE id = ?2
                ",
                params![sort_order, task_id, now],
            )?;
        }

        Ok(())
    }

    pub fn reorder_task(&self, id: i64, position: i64) -> Result<Option<Task>, AppError> {
        let Some(task) = self.get_task(id)? else {
            return Ok(None);
        };

        if task.status != TaskStatus::Pending {
            return Err(AppError::User(format!(
                "Task is not pending and cannot be reordered: {id}"
            )));
        }

        let mut statement = self.conn.prepare(
            "
            SELECT id
            FROM tasks
            WHERE project_id = ?1
              AND status = 'pending'
            ORDER BY sort_order, id
            ",
        )?;
        let mut ids = statement
            .query_map(params![task.project_id], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;

        let task_count = i64::try_from(ids.len()).unwrap_or(i64::MAX);
        if position > task_count {
            return Err(AppError::User(format!(
                "Invalid reorder position: {position}; valid range is 1..={task_count}"
            )));
        }

        ids.retain(|task_id| *task_id != id);
        let target_index = usize::try_from(position - 1).unwrap_or(usize::MAX);
        ids.insert(target_index, id);

        let now = now_timestamp();
        for (index, task_id) in ids.iter().enumerate() {
            self.conn.execute(
                "
                UPDATE tasks
                SET sort_order = ?1,
                    updated_at = CASE WHEN id = ?2 THEN ?3 ELSE updated_at END,
                    sync_dirty = CASE WHEN id = ?2 THEN 1 ELSE sync_dirty END
                WHERE id = ?2
                ",
                params![i64::try_from(index + 1).unwrap_or(i64::MAX), task_id, now],
            )?;
        }

        self.get_task(id)
    }

    fn visible_id_mapping_path(&self) -> PathBuf {
        visible_id_mapping_path(&self.db_path)
    }

    pub fn active_project_names(&self) -> Result<Vec<String>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT p.name
            FROM projects p
            WHERE p.name_normalized = 'inbox'
               OR (
                    p.archived_at IS NULL
                    AND EXISTS (
                        SELECT 1
                        FROM tasks t
                        WHERE t.project_id = p.id
                          AND t.status IN ('pending', 'done')
                    )
               )
            ORDER BY CASE WHEN p.name_normalized = 'inbox' THEN 0 ELSE 1 END,
                     p.name COLLATE NOCASE
            ",
        )?;

        let names = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    pub fn export_sync_request(&self) -> Result<SyncRequest, AppError> {
        let transaction = self.conn.unchecked_transaction()?;
        let request = self.read_sync_state()?;
        transaction.commit()?;
        Ok(request)
    }

    // Call only inside a transaction so all queries see the same snapshot.
    fn read_sync_state(&self) -> Result<SyncRequest, AppError> {
        Ok(SyncRequest {
            checkpoint: self.sync_checkpoint()?,
            projects: self.sync_projects()?,
            tasks: self.sync_tasks()?,
            tombstones: self.sync_tombstones()?,
        })
    }

    pub fn apply_sync_response(
        &self,
        sent: &SyncRequest,
        response: &SyncResponse,
    ) -> Result<(), AppError> {
        self.in_immediate_transaction(|| {
            if self.read_sync_state()? != *sent {
                return Err(AppError::SyncConflict(
                    "Local data changed during sync; the response was not applied. Run ct sync again."
                        .to_string(),
                ));
            }
            for project in &response.projects {
                if !self.has_newer_or_equal_tombstone(
                    SyncEntityKind::Project,
                    &project.uuid,
                    project.sync_revision,
                )? {
                    self.upsert_sync_project(project)?;
                }
            }
            for task in &response.tasks {
                if !self.has_newer_or_equal_tombstone(
                    SyncEntityKind::Task,
                    &task.uuid,
                    task.sync_revision,
                )? {
                    self.upsert_sync_task(task)?;
                }
            }
            for tombstone in &response.tombstones {
                self.apply_sync_tombstone(tombstone)?;
            }
            self.set_sync_checkpoint(response.checkpoint)
        })
    }

    pub fn apply_server_sync(&self, request: &SyncRequest) -> Result<SyncResponse, AppError> {
        self.in_immediate_transaction(|| {
            self.validate_sync_request(request)?;
            let bootstrap = request.checkpoint > 0 && self.is_pristine_sync_server()?;
            let server_checkpoint = self.max_sync_revision()?;
            if !bootstrap && request.checkpoint > server_checkpoint {
                return Err(AppError::SyncConflict(format!(
                    "client checkpoint {} is ahead of server checkpoint {}; sync another current client or rebuild the server",
                    request.checkpoint, server_checkpoint
                )));
            }
            let revision_floor = if bootstrap { request.checkpoint } else { 0 };
            let mut project_map = HashMap::new();
            let mut forced_projects = HashSet::new();
            let mut forced_tasks = HashSet::new();
            let mut forced_tombstones = HashSet::new();
            let mut touched_projects = HashSet::new();

            // Resolve the complete project snapshot before accepting any tasks. A server UUID is
            // canonical once it exists; same-name client UUIDs are aliases for it.
            for project in &request.projects {
                if self.sync_tombstone_by_key(SyncEntityKind::Project, &project.uuid)?.is_some() {
                    project_map.insert(project.uuid.clone(), None);
                    forced_tombstones
                        .insert((SyncEntityKind::Project.as_str(), project.uuid.clone()));
                    continue;
                }

                let by_uuid = self.sync_project_by_uuid(&project.uuid)?;
                let by_name = self.sync_project_by_name(&project.name_normalized)?;
                if let (Some(known), Some(named)) = (&by_uuid, &by_name)
                    && known.uuid != named.uuid
                {
                    return Err(AppError::SyncConflict(format!(
                        "project {} cannot be renamed to normalized name {:?} because it belongs to project {}; choose a distinct project name",
                        project.uuid, project.name_normalized, named.uuid
                    )));
                }

                if let Some(existing) = by_uuid {
                    project_map.insert(project.uuid.clone(), Some(existing.uuid.clone()));
                    if bootstrap || project.sync_dirty {
                        if sync_project_contents_equal(&existing, project) {
                            self.mark_sync_project_clean(&existing.uuid)?;
                            forced_projects.insert(existing.uuid);
                        } else {
                            let mut accepted = project.clone();
                            accepted.uuid = existing.uuid.clone();
                            accepted.sync_revision =
                                self.next_sync_revision_after(revision_floor)?;
                            accepted.sync_dirty = false;
                            self.update_sync_project_by_uuid(&accepted)?;
                        }
                    }
                } else if let Some(existing) = by_name {
                    project_map.insert(project.uuid.clone(), Some(existing.uuid.clone()));
                    self.mark_sync_project_clean(&existing.uuid)?;
                    forced_projects.insert(existing.uuid);
                } else {
                    let mut accepted = project.clone();
                    accepted.sync_revision = self.next_sync_revision_after(revision_floor)?;
                    accepted.sync_dirty = false;
                    self.insert_sync_project(&accepted)?;
                    project_map.insert(project.uuid.clone(), Some(accepted.uuid));
                }
            }

            // Apply task deletions first, followed by project deletions. This makes an incoming
            // project deletion authoritative over every descendant in the same request.
            for entity_type in [SyncEntityKind::Task, SyncEntityKind::Project] {
                for tombstone in request.tombstones.iter().filter(|tombstone| {
                    tombstone.entity_type == entity_type
                        && (bootstrap || tombstone.sync_dirty)
                }) {
                    if let Some(existing) = self
                        .sync_tombstone_by_key(tombstone.entity_type, &tombstone.uuid)?
                        && sync_tombstone_contents_equal(&existing, tombstone)
                    {
                        forced_tombstones.insert((
                            tombstone.entity_type.as_str(),
                            tombstone.uuid.clone(),
                        ));
                        continue;
                    }

                    let revision = self.next_sync_revision_after(revision_floor)?;
                    let mut accepted = tombstone.clone();
                    accepted.sync_revision = revision;
                    accepted.sync_dirty = false;
                    match accepted.entity_type {
                        SyncEntityKind::Task => {
                            if let Some(project_id) = self.task_project_id(&accepted.uuid)? {
                                touched_projects.insert(project_id);
                            }
                            self.apply_sync_tombstone(&accepted)?;
                        }
                        SyncEntityKind::Project => {
                            self.apply_server_project_tombstone(&accepted)?;
                        }
                    }
                    forced_tombstones
                        .insert((accepted.entity_type.as_str(), accepted.uuid));
                }
            }

            for task in request
                .tasks
                .iter()
                .filter(|task| bootstrap || task.sync_dirty)
            {
                if self
                    .sync_tombstone_by_key(SyncEntityKind::Task, &task.uuid)?
                    .is_some()
                {
                    forced_tombstones
                        .insert((SyncEntityKind::Task.as_str(), task.uuid.clone()));
                    continue;
                }

                let canonical_project_uuid = match project_map.get(&task.project_uuid) {
                    Some(Some(uuid)) => uuid.clone(),
                    Some(None) => {
                        let project_tombstone = self
                            .sync_tombstone_by_key(
                                SyncEntityKind::Project,
                                &task.project_uuid,
                            )?
                            .expect("tombstoned project in project map");
                        forced_tombstones.insert((
                            SyncEntityKind::Project.as_str(),
                            task.project_uuid.clone(),
                        ));
                        self.tombstone_task_rejected_by_project(
                            &task.uuid,
                            &project_tombstone,
                            revision_floor,
                            &mut touched_projects,
                            &mut forced_tombstones,
                        )?;
                        continue;
                    }
                    None => {
                        if let Some(project_tombstone) = self
                            .sync_tombstone_by_key(
                                SyncEntityKind::Project,
                                &task.project_uuid,
                            )?
                        {
                            forced_tombstones.insert((
                                SyncEntityKind::Project.as_str(),
                                task.project_uuid.clone(),
                            ));
                            self.tombstone_task_rejected_by_project(
                                &task.uuid,
                                &project_tombstone,
                                revision_floor,
                                &mut touched_projects,
                                &mut forced_tombstones,
                            )?;
                            continue;
                        }
                        if self.sync_project_by_uuid(&task.project_uuid)?.is_some() {
                            task.project_uuid.clone()
                        } else {
                            return Err(orphan_task_conflict(task));
                        }
                    }
                };

                if let Some(project_tombstone) = self
                    .sync_tombstone_by_key(
                        SyncEntityKind::Project,
                        &canonical_project_uuid,
                    )?
                {
                    forced_tombstones.insert((
                        SyncEntityKind::Project.as_str(),
                        canonical_project_uuid,
                    ));
                    self.tombstone_task_rejected_by_project(
                        &task.uuid,
                        &project_tombstone,
                        revision_floor,
                        &mut touched_projects,
                        &mut forced_tombstones,
                    )?;
                    continue;
                }

                let existing = self.sync_task_by_uuid(&task.uuid)?;
                let mut accepted = task.clone();
                accepted.project_uuid = canonical_project_uuid;
                if existing
                    .as_ref()
                    .is_some_and(|current| sync_task_contents_equal(current, &accepted))
                {
                    forced_tasks.insert(task.uuid.clone());
                    continue;
                }

                if let Some(project_id) = self.task_project_id(&task.uuid)? {
                    touched_projects.insert(project_id);
                }
                let destination_id = self.project_id_by_uuid(&accepted.project_uuid)?;
                touched_projects.insert(destination_id);
                accepted.sync_revision = self.next_sync_revision_after(revision_floor)?;
                accepted.sync_dirty = false;
                self.upsert_sync_task(&accepted)?;
            }

            self.canonicalize_pending_sort_orders(&touched_projects, revision_floor)?;
            let checkpoint = self.max_sync_revision()?;
            self.set_sync_checkpoint(checkpoint)?;
            self.build_sync_response(
                request.checkpoint,
                checkpoint,
                &forced_projects,
                &forced_tasks,
                &forced_tombstones,
            )
        })
    }

    fn validate_sync_request(&self, request: &SyncRequest) -> Result<(), AppError> {
        if request.checkpoint < 0 {
            return Err(AppError::SyncBadRequest(format!(
                "checkpoint must not be negative: {}",
                request.checkpoint
            )));
        }

        let mut entity_keys = HashSet::new();
        let mut project_names = HashSet::new();
        for project in &request.projects {
            if project.sync_revision < 0 {
                return Err(AppError::SyncBadRequest(format!(
                    "project {} has a negative revision",
                    project.uuid
                )));
            }
            let normalized = normalize_project_name(&project.name);
            if normalized.is_empty() || project.name_normalized != normalized {
                return Err(AppError::SyncBadRequest(format!(
                    "project {} has an invalid normalized name",
                    project.uuid
                )));
            }
            if !entity_keys.insert((SyncEntityKind::Project.as_str(), project.uuid.as_str())) {
                return Err(AppError::SyncBadRequest(format!(
                    "duplicate project entity key: {}",
                    project.uuid
                )));
            }
            if !project_names.insert(project.name_normalized.as_str()) {
                return Err(AppError::SyncBadRequest(format!(
                    "duplicate normalized project name in snapshot: {:?}",
                    project.name_normalized
                )));
            }
        }

        let mut pending_counts = HashMap::new();
        for task in &request.tasks {
            if task.status == TaskStatus::Pending.as_str() {
                *pending_counts
                    .entry(task.project_uuid.as_str())
                    .or_insert(0usize) += 1;
            }
        }

        for task in &request.tasks {
            if task.sync_revision < 0 {
                return Err(AppError::SyncBadRequest(format!(
                    "task {} has a negative revision",
                    task.uuid
                )));
            }
            let status = TaskStatus::try_from(task.status.as_str()).map_err(|_| {
                AppError::SyncBadRequest(format!(
                    "task {} has invalid status {:?}",
                    task.uuid, task.status
                ))
            })?;
            if status == TaskStatus::Pending {
                let pending_count = pending_counts
                    .get(task.project_uuid.as_str())
                    .copied()
                    .unwrap_or(0);
                if task.sort_order <= 0
                    || usize::try_from(task.sort_order)
                        .map_or(true, |position| position > pending_count)
                {
                    return Err(AppError::SyncBadRequest(format!(
                        "pending task {} has invalid sort position {}; valid range for project {} is 1..={}",
                        task.uuid, task.sort_order, task.project_uuid, pending_count
                    )));
                }
            }
            if !entity_keys.insert((SyncEntityKind::Task.as_str(), task.uuid.as_str())) {
                return Err(AppError::SyncBadRequest(format!(
                    "duplicate task entity key: {}",
                    task.uuid
                )));
            }
        }

        for tombstone in &request.tombstones {
            if tombstone.sync_revision < 0 {
                return Err(AppError::SyncBadRequest(format!(
                    "{} tombstone {} has a negative revision",
                    tombstone.entity_type.as_str(),
                    tombstone.uuid
                )));
            }
            if !entity_keys.insert((tombstone.entity_type.as_str(), tombstone.uuid.as_str())) {
                return Err(AppError::SyncBadRequest(format!(
                    "duplicate {} entity key: {}",
                    tombstone.entity_type.as_str(),
                    tombstone.uuid
                )));
            }
        }
        Ok(())
    }

    fn build_sync_response(
        &self,
        request_checkpoint: i64,
        checkpoint: i64,
        forced_projects: &HashSet<String>,
        forced_tasks: &HashSet<String>,
        forced_tombstones: &HashSet<(&'static str, String)>,
    ) -> Result<SyncResponse, AppError> {
        let mut projects = self.sync_projects_since(request_checkpoint)?;
        for uuid in forced_projects {
            if !projects.iter().any(|project| project.uuid == *uuid)
                && let Some(project) = self.sync_project_by_uuid(uuid)?
            {
                projects.push(project);
            }
        }
        projects.sort_by(|left, right| {
            (left.sync_revision, &left.name_normalized, &left.uuid).cmp(&(
                right.sync_revision,
                &right.name_normalized,
                &right.uuid,
            ))
        });

        let mut tasks = self.sync_tasks_since(request_checkpoint)?;
        for uuid in forced_tasks {
            if !tasks.iter().any(|task| task.uuid == *uuid)
                && let Some(task) = self.sync_task_by_uuid(uuid)?
            {
                tasks.push(task);
            }
        }
        tasks.sort_by(|left, right| {
            (
                left.sync_revision,
                &left.project_uuid,
                left.sort_order,
                &left.uuid,
            )
                .cmp(&(
                    right.sync_revision,
                    &right.project_uuid,
                    right.sort_order,
                    &right.uuid,
                ))
        });

        let mut tombstones = self.sync_tombstones_since(request_checkpoint)?;
        for (entity_type, uuid) in forced_tombstones {
            if !tombstones.iter().any(|tombstone| {
                tombstone.entity_type.as_str() == *entity_type && tombstone.uuid == *uuid
            }) {
                let entity_type =
                    SyncEntityKind::try_from(*entity_type).map_err(AppError::StorageMessage)?;
                if let Some(tombstone) = self.sync_tombstone_by_key(entity_type, uuid)? {
                    tombstones.push(tombstone);
                }
            }
        }
        tombstones.sort_by(|left, right| {
            (left.sync_revision, left.entity_type.as_str(), &left.uuid).cmp(&(
                right.sync_revision,
                right.entity_type.as_str(),
                &right.uuid,
            ))
        });

        Ok(SyncResponse {
            checkpoint,
            projects,
            tasks,
            tombstones,
        })
    }

    fn sync_project_by_uuid(&self, uuid: &str) -> Result<Option<SyncProject>, AppError> {
        self.conn
            .query_row(
                "
                SELECT uuid, name, name_normalized, created_at, updated_at, archived_at,
                       sync_revision, sync_dirty
                FROM projects
                WHERE uuid = ?1
                ",
                params![uuid],
                sync_project_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    fn sync_project_by_name(&self, normalized: &str) -> Result<Option<SyncProject>, AppError> {
        self.conn
            .query_row(
                "
                SELECT uuid, name, name_normalized, created_at, updated_at, archived_at,
                       sync_revision, sync_dirty
                FROM projects
                WHERE name_normalized = ?1
                ",
                params![normalized],
                sync_project_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    fn sync_task_by_uuid(&self, uuid: &str) -> Result<Option<SyncTask>, AppError> {
        self.conn
            .query_row(
                "
                SELECT t.uuid, p.uuid, t.sort_order, t.title, t.notes, t.status,
                       t.created_at, t.updated_at, t.completed_at, t.sync_revision,
                       t.sync_dirty
                FROM tasks t
                JOIN projects p ON p.id = t.project_id
                WHERE t.uuid = ?1
                ",
                params![uuid],
                sync_task_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    fn sync_tombstone_by_key(
        &self,
        entity_type: SyncEntityKind,
        uuid: &str,
    ) -> Result<Option<SyncTombstone>, AppError> {
        self.conn
            .query_row(
                "
                SELECT entity_type, uuid, deleted_at, sync_revision, sync_dirty
                FROM sync_tombstones
                WHERE entity_type = ?1 AND uuid = ?2
                ",
                params![entity_type.as_str(), uuid],
                sync_tombstone_from_row,
            )
            .optional()
            .map_err(AppError::from)
    }

    fn update_sync_project_by_uuid(&self, project: &SyncProject) -> Result<(), AppError> {
        self.conn.execute(
            "
            UPDATE projects
            SET name = ?1,
                name_normalized = ?2,
                created_at = ?3,
                updated_at = ?4,
                archived_at = ?5,
                sync_revision = ?6,
                sync_dirty = ?7
            WHERE uuid = ?8
            ",
            params![
                project.name,
                project.name_normalized,
                project.created_at,
                project.updated_at,
                project.archived_at,
                project.sync_revision,
                if project.sync_dirty { 1 } else { 0 },
                project.uuid
            ],
        )?;
        Ok(())
    }

    fn mark_sync_project_clean(&self, uuid: &str) -> Result<(), AppError> {
        self.conn.execute(
            "UPDATE projects SET sync_dirty = 0 WHERE uuid = ?1",
            params![uuid],
        )?;
        Ok(())
    }

    fn insert_sync_project(&self, project: &SyncProject) -> Result<(), AppError> {
        self.conn.execute(
            "
            INSERT INTO projects (
              uuid, name, name_normalized, created_at, updated_at, archived_at,
              sync_revision, sync_dirty
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
            params![
                project.uuid,
                project.name,
                project.name_normalized,
                project.created_at,
                project.updated_at,
                project.archived_at,
                project.sync_revision,
                if project.sync_dirty { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    fn task_project_id(&self, task_uuid: &str) -> Result<Option<i64>, AppError> {
        self.conn
            .query_row(
                "SELECT project_id FROM tasks WHERE uuid = ?1",
                params![task_uuid],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::from)
    }

    fn tombstone_task_rejected_by_project(
        &self,
        task_uuid: &str,
        project_tombstone: &SyncTombstone,
        revision_floor: i64,
        touched_projects: &mut HashSet<i64>,
        forced_tombstones: &mut HashSet<(&'static str, String)>,
    ) -> Result<(), AppError> {
        let Some(project_id) = self.task_project_id(task_uuid)? else {
            return Ok(());
        };

        touched_projects.insert(project_id);
        let task_tombstone = SyncTombstone {
            entity_type: SyncEntityKind::Task,
            uuid: task_uuid.to_string(),
            deleted_at: project_tombstone.deleted_at.clone(),
            sync_revision: self.next_sync_revision_after(revision_floor)?,
            sync_dirty: false,
        };
        self.apply_sync_tombstone(&task_tombstone)?;
        forced_tombstones.insert((SyncEntityKind::Task.as_str(), task_uuid.to_string()));
        Ok(())
    }

    fn apply_server_project_tombstone(&self, tombstone: &SyncTombstone) -> Result<(), AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT t.uuid
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE p.uuid = ?1
            ",
        )?;
        let task_uuids = statement
            .query_map(params![tombstone.uuid], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        for uuid in task_uuids {
            let child_tombstone = SyncTombstone {
                entity_type: SyncEntityKind::Task,
                uuid,
                deleted_at: tombstone.deleted_at.clone(),
                sync_revision: tombstone.sync_revision,
                sync_dirty: false,
            };
            self.apply_sync_tombstone(&child_tombstone)?;
        }
        self.apply_sync_tombstone(tombstone)
    }

    fn canonicalize_pending_sort_orders(
        &self,
        project_ids: &HashSet<i64>,
        revision_floor: i64,
    ) -> Result<(), AppError> {
        let mut project_ids = project_ids.iter().copied().collect::<Vec<_>>();
        project_ids.sort_unstable();
        for project_id in project_ids {
            let mut statement = self.conn.prepare(
                "
                SELECT id, sort_order
                FROM tasks
                WHERE project_id = ?1 AND status = 'pending'
                ORDER BY sort_order, uuid
                ",
            )?;
            let tasks = statement
                .query_map(params![project_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);

            for (index, (task_id, current_order)) in tasks.into_iter().enumerate() {
                let canonical_order = i64::try_from(index + 1).unwrap_or(i64::MAX);
                if current_order != canonical_order {
                    let revision = self.next_sync_revision_after(revision_floor)?;
                    self.conn.execute(
                        "
                        UPDATE tasks
                        SET sort_order = ?1, sync_revision = ?2, sync_dirty = 0
                        WHERE id = ?3
                        ",
                        params![canonical_order, revision, task_id],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn is_pristine_sync_server(&self) -> Result<bool, AppError> {
        if self.sync_checkpoint()? != 0 {
            return Ok(false);
        }

        let is_pristine = self.conn.query_row(
            "
            SELECT CASE WHEN
                (SELECT COUNT(*) FROM projects) = 1
                AND EXISTS (
                    SELECT 1 FROM projects
                    WHERE name_normalized = 'inbox' AND sync_revision = 0
                )
                AND NOT EXISTS (SELECT 1 FROM tasks)
                AND NOT EXISTS (SELECT 1 FROM sync_tombstones)
            THEN 1 ELSE 0 END
            ",
            [],
            |row| row.get::<_, i64>(0),
        )? != 0;
        Ok(is_pristine)
    }

    fn sync_checkpoint(&self) -> Result<i64, AppError> {
        self.conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'checkpoint'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .map_or(Ok(0), Ok)
    }

    fn set_sync_checkpoint(&self, checkpoint: i64) -> Result<(), AppError> {
        self.conn.execute(
            "
            INSERT INTO sync_state (key, value) VALUES ('checkpoint', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
            params![checkpoint],
        )?;
        Ok(())
    }

    fn sync_projects(&self) -> Result<Vec<SyncProject>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT uuid, name, name_normalized, created_at, updated_at, archived_at,
                   sync_revision, sync_dirty
            FROM projects
            ORDER BY name_normalized
            ",
        )?;
        statement
            .query_map([], |row| {
                Ok(SyncProject {
                    uuid: row.get(0)?,
                    name: row.get(1)?,
                    name_normalized: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    archived_at: row.get(5)?,
                    sync_revision: row.get(6)?,
                    sync_dirty: row.get::<_, i64>(7)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn sync_tasks(&self) -> Result<Vec<SyncTask>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT t.uuid, p.uuid, t.sort_order, t.title, t.notes, t.status,
                   t.created_at, t.updated_at, t.completed_at, t.sync_revision, t.sync_dirty
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            ORDER BY p.name_normalized, t.sort_order, t.id
            ",
        )?;
        statement
            .query_map([], |row| {
                Ok(SyncTask {
                    uuid: row.get(0)?,
                    project_uuid: row.get(1)?,
                    sort_order: row.get(2)?,
                    title: row.get(3)?,
                    notes: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    completed_at: row.get(8)?,
                    sync_revision: row.get(9)?,
                    sync_dirty: row.get::<_, i64>(10)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn sync_tombstones(&self) -> Result<Vec<SyncTombstone>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT entity_type, uuid, deleted_at, sync_revision, sync_dirty
            FROM sync_tombstones
            ORDER BY sync_revision, entity_type, uuid
            ",
        )?;
        statement
            .query_map([], sync_tombstone_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn sync_projects_since(&self, checkpoint: i64) -> Result<Vec<SyncProject>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT uuid, name, name_normalized, created_at, updated_at, archived_at,
                   sync_revision, sync_dirty
            FROM projects
            WHERE sync_revision > ?1
              AND NOT EXISTS (
                SELECT 1 FROM sync_tombstones st
                WHERE st.entity_type = 'project' AND st.uuid = projects.uuid
              )
            ORDER BY sync_revision, name_normalized
            ",
        )?;
        statement
            .query_map(params![checkpoint], sync_project_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn sync_tasks_since(&self, checkpoint: i64) -> Result<Vec<SyncTask>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT t.uuid, p.uuid, t.sort_order, t.title, t.notes, t.status,
                   t.created_at, t.updated_at, t.completed_at, t.sync_revision, t.sync_dirty
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            WHERE t.sync_revision > ?1
              AND NOT EXISTS (
                SELECT 1 FROM sync_tombstones st
                WHERE st.entity_type = 'task' AND st.uuid = t.uuid
              )
            ORDER BY t.sync_revision, p.name_normalized, t.sort_order, t.id
            ",
        )?;
        statement
            .query_map(params![checkpoint], sync_task_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn sync_tombstones_since(&self, checkpoint: i64) -> Result<Vec<SyncTombstone>, AppError> {
        let mut statement = self.conn.prepare(
            "
            SELECT entity_type, uuid, deleted_at, sync_revision, sync_dirty
            FROM sync_tombstones
            WHERE sync_revision > ?1
            ORDER BY sync_revision, entity_type, uuid
            ",
        )?;
        statement
            .query_map(params![checkpoint], sync_tombstone_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    fn upsert_sync_project(&self, project: &SyncProject) -> Result<(), AppError> {
        let existing_id = self
            .conn
            .query_row(
                "SELECT id FROM projects WHERE uuid = ?1 OR name_normalized = ?2 ORDER BY uuid = ?1 DESC LIMIT 1",
                params![project.uuid, project.name_normalized],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;

        match existing_id {
            Some(id) => {
                self.conn.execute(
                    "
                    UPDATE projects
                    SET uuid = ?1,
                        name = ?2,
                        name_normalized = ?3,
                        created_at = ?4,
                        updated_at = ?5,
                        archived_at = ?6,
                        sync_revision = ?7,
                        sync_dirty = ?8
                    WHERE id = ?9
                    ",
                    params![
                        project.uuid,
                        project.name,
                        project.name_normalized,
                        project.created_at,
                        project.updated_at,
                        project.archived_at,
                        project.sync_revision,
                        if project.sync_dirty { 1 } else { 0 },
                        id
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "
                    INSERT INTO projects (
                      uuid, name, name_normalized, created_at, updated_at, archived_at,
                      sync_revision, sync_dirty
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ",
                    params![
                        project.uuid,
                        project.name,
                        project.name_normalized,
                        project.created_at,
                        project.updated_at,
                        project.archived_at,
                        project.sync_revision,
                        if project.sync_dirty { 1 } else { 0 }
                    ],
                )?;
            }
        }
        Ok(())
    }

    fn upsert_sync_task(&self, task: &SyncTask) -> Result<(), AppError> {
        let project_id = self.project_id_by_uuid(&task.project_uuid)?;
        self.conn.execute(
            "
            INSERT INTO tasks (
              uuid, project_id, sort_order, title, notes, status, created_at, updated_at,
              completed_at, sync_revision, sync_dirty
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(uuid) DO UPDATE SET
              project_id = excluded.project_id,
              sort_order = excluded.sort_order,
              title = excluded.title,
              notes = excluded.notes,
              status = excluded.status,
              created_at = excluded.created_at,
              updated_at = excluded.updated_at,
              completed_at = excluded.completed_at,
              sync_revision = excluded.sync_revision,
              sync_dirty = excluded.sync_dirty
            ",
            params![
                task.uuid,
                project_id,
                task.sort_order,
                task.title,
                task.notes,
                task.status,
                task.created_at,
                task.updated_at,
                task.completed_at,
                task.sync_revision,
                if task.sync_dirty { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    fn apply_sync_tombstone(&self, tombstone: &SyncTombstone) -> Result<(), AppError> {
        self.conn.execute(
            "
            INSERT INTO sync_tombstones (entity_type, uuid, deleted_at, sync_revision, sync_dirty)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(entity_type, uuid) DO UPDATE SET
              deleted_at = excluded.deleted_at,
              sync_revision = excluded.sync_revision,
              sync_dirty = excluded.sync_dirty
            ",
            params![
                tombstone.entity_type.as_str(),
                tombstone.uuid,
                tombstone.deleted_at,
                tombstone.sync_revision,
                if tombstone.sync_dirty { 1 } else { 0 }
            ],
        )?;

        match tombstone.entity_type {
            SyncEntityKind::Project => {
                self.conn.execute(
                    "
                    DELETE FROM tasks
                    WHERE project_id IN (SELECT id FROM projects WHERE uuid = ?1)
                    ",
                    params![tombstone.uuid],
                )?;
                self.conn.execute(
                    "DELETE FROM projects WHERE uuid = ?1",
                    params![tombstone.uuid],
                )?;
            }
            SyncEntityKind::Task => {
                self.conn
                    .execute("DELETE FROM tasks WHERE uuid = ?1", params![tombstone.uuid])?;
            }
        }
        Ok(())
    }

    fn project_id_by_uuid(&self, uuid: &str) -> Result<i64, AppError> {
        self.conn
            .query_row(
                "SELECT id FROM projects WHERE uuid = ?1",
                params![uuid],
                |row| row.get(0),
            )
            .map_err(AppError::from)
    }

    fn max_sync_revision(&self) -> Result<i64, AppError> {
        self.conn
            .query_row(
                "
                SELECT MAX(revision) FROM (
                  SELECT COALESCE(MAX(sync_revision), 0) AS revision FROM projects
                  UNION ALL SELECT COALESCE(MAX(sync_revision), 0) FROM tasks
                  UNION ALL SELECT COALESCE(MAX(sync_revision), 0) FROM sync_tombstones
                )
                ",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )?
            .map_or(Ok(0), Ok)
    }

    fn next_sync_revision_after(&self, floor: i64) -> Result<i64, AppError> {
        Ok(self.max_sync_revision()?.max(floor) + 1)
    }

    fn completed_task_tombstone_rows(
        &self,
        project_id: Option<i64>,
    ) -> Result<Vec<(String, String)>, AppError> {
        let now = now_timestamp();
        let sql = match project_id {
            Some(_) => "SELECT uuid FROM tasks WHERE status = 'done' AND project_id = ?1",
            None => "SELECT uuid FROM tasks WHERE status = 'done'",
        };
        let mut statement = self.conn.prepare(sql)?;
        let rows = match project_id {
            Some(project_id) => statement
                .query_map(params![project_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
            None => statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows.into_iter().map(|uuid| (uuid, now.clone())).collect())
    }

    fn record_tombstone(
        &self,
        entity_type: SyncEntityKind,
        uuid: &str,
        deleted_at: &str,
    ) -> Result<(), AppError> {
        self.conn.execute(
            "
            INSERT INTO sync_tombstones (entity_type, uuid, deleted_at, sync_revision, sync_dirty)
            VALUES (?1, ?2, ?3, 0, 1)
            ON CONFLICT(entity_type, uuid) DO UPDATE SET
              deleted_at = excluded.deleted_at,
              sync_revision = 0,
              sync_dirty = 1
            ",
            params![entity_type.as_str(), uuid, deleted_at],
        )?;
        Ok(())
    }

    fn has_newer_or_equal_tombstone(
        &self,
        entity_type: SyncEntityKind,
        uuid: &str,
        sync_revision: i64,
    ) -> Result<bool, AppError> {
        let tombstone_revision = self
            .conn
            .query_row(
                "
                SELECT sync_revision
                FROM sync_tombstones
                WHERE entity_type = ?1 AND uuid = ?2
                ",
                params![entity_type.as_str(), uuid],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(tombstone_revision.is_some_and(|revision| revision >= sync_revision))
    }

    fn in_immediate_transaction<T>(
        &self,
        operation: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match operation() {
            Ok(value) => {
                if let Err(error) = self.conn.execute_batch("COMMIT").map_err(AppError::from) {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(error)
                } else {
                    Ok(value)
                }
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
}

pub fn database_path() -> Result<PathBuf, AppError> {
    database_path_from(
        env::var_os("CT_DB").as_deref(),
        env::var_os("XDG_DATA_HOME").as_deref(),
        dirs::home_dir(),
    )
}

fn visible_id_mapping_path(database_path: &Path) -> PathBuf {
    database_path.with_extension("list-ids.json")
}

fn database_path_from(
    ct_db: Option<&OsStr>,
    xdg_data_home: Option<&OsStr>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = ct_db.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    if let Some(path) = xdg_data_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join("couch-tasks").join("tasks.db"));
    }

    let home = home_dir.ok_or_else(|| {
        AppError::StorageMessage("could not determine home directory for database path".to_string())
    })?;
    Ok(home
        .join(".local")
        .join("share")
        .join("couch-tasks")
        .join("tasks.db"))
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        name_normalized: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        archived_at: row.get(6)?,
    })
}

fn sync_project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncProject> {
    Ok(SyncProject {
        uuid: row.get(0)?,
        name: row.get(1)?,
        name_normalized: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        archived_at: row.get(5)?,
        sync_revision: row.get(6)?,
        sync_dirty: row.get::<_, i64>(7)? != 0,
    })
}

fn sync_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncTask> {
    Ok(SyncTask {
        uuid: row.get(0)?,
        project_uuid: row.get(1)?,
        sort_order: row.get(2)?,
        title: row.get(3)?,
        notes: row.get(4)?,
        status: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        completed_at: row.get(8)?,
        sync_revision: row.get(9)?,
        sync_dirty: row.get::<_, i64>(10)? != 0,
    })
}

fn sync_tombstone_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncTombstone> {
    let entity_type: String = row.get(0)?;
    let entity_type = SyncEntityKind::try_from(entity_type.as_str()).map_err(|message| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            )),
        )
    })?;

    Ok(SyncTombstone {
        entity_type,
        uuid: row.get(1)?,
        deleted_at: row.get(2)?,
        sync_revision: row.get(3)?,
        sync_dirty: row.get::<_, i64>(4)? != 0,
    })
}

fn sync_project_contents_equal(left: &SyncProject, right: &SyncProject) -> bool {
    left.uuid == right.uuid
        && left.name == right.name
        && left.name_normalized == right.name_normalized
        && left.created_at == right.created_at
        && left.updated_at == right.updated_at
        && left.archived_at == right.archived_at
}

fn sync_task_contents_equal(left: &SyncTask, right: &SyncTask) -> bool {
    left.uuid == right.uuid
        && left.project_uuid == right.project_uuid
        && left.sort_order == right.sort_order
        && left.title == right.title
        && left.notes == right.notes
        && left.status == right.status
        && left.created_at == right.created_at
        && left.updated_at == right.updated_at
        && left.completed_at == right.completed_at
}

fn sync_tombstone_contents_equal(left: &SyncTombstone, right: &SyncTombstone) -> bool {
    left.entity_type == right.entity_type
        && left.uuid == right.uuid
        && left.deleted_at == right.deleted_at
}

fn orphan_task_conflict(task: &SyncTask) -> AppError {
    AppError::SyncConflict(format!(
        "task {} references missing project {}; repair it with `ct update {} --project <valid-project>` or delete it with `ct delete {}`",
        task.uuid, task.project_uuid, task.uuid, task.uuid
    ))
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get(7)?;
    let status = TaskStatus::try_from(status.as_str()).map_err(|message| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            )),
        )
    })?;

    Ok(Task {
        id: row.get(0)?,
        uuid: row.get(1)?,
        project_id: row.get(2)?,
        project_name: row.get(3)?,
        sort_order: row.get(4)?,
        title: row.get(5)?,
        notes: row.get(6)?,
        status,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        completed_at: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_sync_responses_preserve_concurrent_changes_and_retry_converges() {
        for change in [
            "edit", "add", "delete", "move", "reorder", "complete", "note",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let client_path = dir.path().join("client.db");
            let client = Storage::open(client_path.clone()).unwrap();
            let other = Storage::open(client_path).unwrap();
            let server = Storage::open(dir.path().join("server.db")).unwrap();
            let task = client.add_task("Work", "original").unwrap();
            client.add_task("Work", "second").unwrap();
            let sent = client.export_sync_request().unwrap();
            let response = server.apply_server_sync(&sent).unwrap();

            // A second connection writes after the server accepted the snapshot,
            // but before the original client applies its response.
            match change {
                "edit" => {
                    other
                        .update_task_details(task.id, "Work", "edited", "new notes")
                        .unwrap();
                }
                "add" => {
                    other.add_task("Home", "concurrent addition").unwrap();
                }
                "delete" => {
                    other.delete_task(task.id).unwrap();
                }
                "move" => {
                    other.move_task(task.id, "Home").unwrap();
                }
                "reorder" => {
                    other.reorder_task(task.id, 2).unwrap();
                }
                "complete" => {
                    other.mark_done(task.id).unwrap();
                }
                "note" => {
                    other.append_note(task.id, "concurrent note").unwrap();
                }
                _ => unreachable!(),
            }
            let current = client.export_sync_request().unwrap();
            let error = client.apply_sync_response(&sent, &response).unwrap_err();
            assert_eq!(error.exit_code(), 4, "{change}");
            assert!(error.to_string().contains("Run ct sync again"));
            assert_eq!(client.export_sync_request().unwrap(), current, "{change}");

            let retry_response = server.apply_server_sync(&current).unwrap();
            client
                .apply_sync_response(&current, &retry_response)
                .unwrap();
            let synced = client.export_sync_request().unwrap();
            let remote = server.export_sync_request().unwrap();
            assert_eq!(synced.tasks, remote.tasks, "{change}");
            assert_eq!(synced.projects, remote.projects, "{change}");
            assert_eq!(synced.tombstones, remote.tombstones, "{change}");
            assert_eq!(synced.checkpoint, retry_response.checkpoint);
            if change == "delete" {
                assert!(client.get_task_by_uuid(&task.uuid).unwrap().is_none());
            } else if change == "edit" {
                assert_eq!(
                    client.get_task_by_uuid(&task.uuid).unwrap().unwrap().title,
                    "edited"
                );
            }
        }
    }

    #[test]
    fn older_sync_response_cannot_overwrite_a_competing_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.db");
        let client = Storage::open(path.clone()).unwrap();
        let other = Storage::open(path).unwrap();
        let server = Storage::open(dir.path().join("server.db")).unwrap();
        client.add_task("Work", "original").unwrap();
        let sent = client.export_sync_request().unwrap();
        let older = server.apply_server_sync(&sent).unwrap();
        other.apply_sync_response(&sent, &older).unwrap();
        other.add_task("Home", "concurrent addition").unwrap();
        let next = other.export_sync_request().unwrap();
        let newer = server.apply_server_sync(&next).unwrap();
        other.apply_sync_response(&next, &newer).unwrap();
        let current = client.export_sync_request().unwrap();
        assert!(client.apply_sync_response(&sent, &older).is_err());
        assert_eq!(client.export_sync_request().unwrap(), current);
    }

    #[test]
    fn database_path_prefers_ct_db() {
        let path = database_path_from(
            Some(OsStr::new("/tmp/custom-tasks.db")),
            Some(OsStr::new("/tmp/xdg")),
            Some(PathBuf::from("/home/example")),
        )
        .expect("database path");

        assert_eq!(path, PathBuf::from("/tmp/custom-tasks.db"));
    }

    #[test]
    fn database_path_uses_xdg_data_home_before_home_fallback() {
        let path = database_path_from(
            None,
            Some(OsStr::new("/tmp/xdg")),
            Some(PathBuf::from("/home/example")),
        )
        .expect("database path");

        assert_eq!(path, PathBuf::from("/tmp/xdg/couch-tasks/tasks.db"));
    }

    #[test]
    fn opening_storage_creates_schema_and_inbox() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");

        let inbox = storage.find_project("inbox").expect("find inbox");
        assert_eq!(inbox.expect("inbox exists").name, INBOX);
    }

    #[test]
    fn project_lookup_is_case_insensitive_and_preserves_first_casing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");

        let home = storage.ensure_project("Home").expect("create Home");
        let lower = storage.ensure_project("home").expect("find home");

        assert_eq!(home.id, lower.id);
        assert_eq!(lower.name, "Home");
    }

    #[test]
    fn active_project_names_exclude_empty_projects_except_inbox() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");

        assert_eq!(
            storage
                .active_project_names()
                .expect("active project names"),
            vec!["inbox".to_string()]
        );

        storage
            .ensure_project("Empty")
            .expect("create empty project");
        storage.add_task("Work", "write tests").expect("add task");
        storage
            .add_task("inbox", "buy milk")
            .expect("add inbox task");

        let names = storage
            .active_project_names()
            .expect("active project names");
        assert_eq!(names, vec!["inbox".to_string(), "Work".to_string()]);
    }

    #[test]
    fn server_sync_assigns_checkpoint_and_clears_dirty_flags() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        client.add_task("Work", "write tests").expect("add task");
        let request = client.export_sync_request().expect("export request");
        let response = server.apply_server_sync(&request).expect("server sync");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &response,
            )
            .expect("apply response");

        assert!(response.checkpoint > 0);
        let request = client.export_sync_request().expect("export after sync");
        assert!(
            request.projects.iter().all(|project| !project.sync_dirty),
            "{request:#?}"
        );
        assert!(request.tasks.iter().all(|task| !task.sync_dirty));
        assert_eq!(request.checkpoint, response.checkpoint);
    }

    #[test]
    fn clean_client_snapshot_bootstraps_a_rebuilt_server() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let old_server_dir = tempfile::tempdir().expect("old server temp dir");
        let new_server_dir = tempfile::tempdir().expect("new server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let old_server =
            Storage::open(old_server_dir.path().join("tasks.db")).expect("open old server");

        client.add_task("Work", "keep task").expect("add kept task");
        let deleted = client
            .add_task("Archive", "delete task")
            .expect("add deleted task");
        let initial_response = old_server
            .apply_server_sync(&client.export_sync_request().expect("export initial"))
            .expect("initial sync");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial_response,
            )
            .expect("apply initial response");

        client.delete_task(deleted.id).expect("delete task");
        let delete_response = old_server
            .apply_server_sync(&client.export_sync_request().expect("export delete"))
            .expect("sync delete");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &delete_response,
            )
            .expect("apply delete response");

        let clean_snapshot = client.export_sync_request().expect("export clean snapshot");
        assert!(clean_snapshot.checkpoint > 0);
        assert!(
            clean_snapshot
                .projects
                .iter()
                .all(|project| !project.sync_dirty)
        );
        assert!(clean_snapshot.tasks.iter().all(|task| !task.sync_dirty));
        assert!(
            clean_snapshot
                .tombstones
                .iter()
                .all(|tombstone| !tombstone.sync_dirty)
        );

        let new_server =
            Storage::open(new_server_dir.path().join("tasks.db")).expect("open new server");
        let rebuild_response = new_server
            .apply_server_sync(&clean_snapshot)
            .expect("bootstrap rebuilt server");

        assert!(rebuild_response.checkpoint > clean_snapshot.checkpoint);
        assert_eq!(
            new_server
                .sync_tasks()
                .expect("new server tasks")
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            vec!["keep task"]
        );
        assert_eq!(
            new_server
                .sync_tombstones()
                .expect("new server tombstones")
                .len(),
            1
        );

        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &rebuild_response,
            )
            .expect("apply rebuilt server response");
        assert_eq!(
            client
                .export_sync_request()
                .expect("export converged client")
                .checkpoint,
            rebuild_response.checkpoint
        );

        client
            .add_task("inbox", "task after rebuild")
            .expect("add task after rebuild");
        let follow_up_response = new_server
            .apply_server_sync(&client.export_sync_request().expect("export follow-up"))
            .expect("sync after rebuild");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &follow_up_response,
            )
            .expect("apply follow-up response");
        assert_eq!(new_server.sync_tasks().expect("new server tasks").len(), 2);
    }

    #[test]
    fn task_deletes_sync_as_tombstones() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        let task = client.add_task("Work", "write tests").expect("add task");
        let response = server
            .apply_server_sync(&client.export_sync_request().expect("export initial"))
            .expect("initial sync");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &response,
            )
            .expect("apply initial response");

        client.delete_task(task.id).expect("delete task");
        let delete_response = server
            .apply_server_sync(&client.export_sync_request().expect("export delete"))
            .expect("delete sync");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &delete_response,
            )
            .expect("apply delete response");

        assert!(client.get_task(task.id).expect("get deleted").is_none());
        assert!(server.sync_tasks().expect("server tasks").is_empty());
        assert_eq!(
            server.sync_tombstones().expect("server tombstones").len(),
            1
        );
    }

    #[test]
    fn two_clients_converge_through_server() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let second_dir = tempfile::tempdir().expect("second temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let second = Storage::open(second_dir.path().join("tasks.db")).expect("open second");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        first.add_task("Work", "write tests").expect("add first");
        second.add_task("Home", "buy milk").expect("add second");

        let first_response = server
            .apply_server_sync(&first.export_sync_request().expect("export first"))
            .expect("sync first");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &first_response,
            )
            .expect("apply first");
        let second_response = server
            .apply_server_sync(&second.export_sync_request().expect("export second"))
            .expect("sync second");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &second_response,
            )
            .expect("apply second");
        let final_first_response = server
            .apply_server_sync(&first.export_sync_request().expect("export first again"))
            .expect("sync first again");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &final_first_response,
            )
            .expect("apply first again");

        let first_titles = first
            .sync_tasks()
            .expect("first tasks")
            .into_iter()
            .map(|task| task.title)
            .collect::<Vec<_>>();
        let second_titles = second
            .sync_tasks()
            .expect("second tasks")
            .into_iter()
            .map(|task| task.title)
            .collect::<Vec<_>>();
        assert_eq!(first_titles, second_titles);
        assert_eq!(
            first_titles,
            vec!["buy milk".to_string(), "write tests".to_string()]
        );
    }

    #[test]
    fn stale_client_edit_does_not_resurrect_deleted_task() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let second_dir = tempfile::tempdir().expect("second temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let second = Storage::open(second_dir.path().join("tasks.db")).expect("open second");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        let task = first.add_task("Work", "write tests").expect("add task");
        let initial_response = server
            .apply_server_sync(&first.export_sync_request().expect("export initial"))
            .expect("sync initial");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &initial_response,
            )
            .expect("apply first initial");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial_response,
            )
            .expect("apply second initial");

        first.delete_task(task.id).expect("delete task");
        let delete_response = server
            .apply_server_sync(&first.export_sync_request().expect("export delete"))
            .expect("sync delete");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &delete_response,
            )
            .expect("apply delete");

        let stale_task_id = second
            .list_pending_tasks()
            .expect("second tasks")
            .first()
            .expect("stale task")
            .id;
        second.mark_done(stale_task_id).expect("edit stale task");
        let stale_response = server
            .apply_server_sync(&second.export_sync_request().expect("export stale edit"))
            .expect("sync stale edit");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &stale_response,
            )
            .expect("apply stale response");

        assert!(server.sync_tasks().expect("server tasks").is_empty());
        assert!(second.sync_tasks().expect("second tasks").is_empty());
        assert_eq!(
            server.sync_tombstones().expect("server tombstones").len(),
            1
        );
    }

    #[test]
    fn sort_order_compaction_syncs_remaining_tasks() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let second_dir = tempfile::tempdir().expect("second temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let second = Storage::open(second_dir.path().join("tasks.db")).expect("open second");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        let first_task = first.add_task("Work", "first").expect("add first task");
        first.add_task("Work", "second").expect("add second task");
        let initial_response = server
            .apply_server_sync(&first.export_sync_request().expect("export initial"))
            .expect("sync initial");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &initial_response,
            )
            .expect("apply first initial");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial_response,
            )
            .expect("apply second initial");

        first.delete_task(first_task.id).expect("delete first task");
        let delete_response = server
            .apply_server_sync(&first.export_sync_request().expect("export delete"))
            .expect("sync delete");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &delete_response,
            )
            .expect("apply delete");

        let tasks = second.list_pending_tasks().expect("second pending tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "second");
        assert_eq!(tasks[0].sort_order, 1);
    }

    #[test]
    fn clean_same_name_project_alias_resolves_before_dirty_task() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let stale_dir = tempfile::tempdir().expect("stale temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let stale = Storage::open(stale_dir.path().join("tasks.db")).expect("open stale");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        first.ensure_project("Work").expect("create canonical Work");
        let canonical_response = server
            .apply_server_sync(&first.export_sync_request().expect("export canonical"))
            .expect("sync canonical");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &canonical_response,
            )
            .expect("apply canonical");
        let canonical_uuid = first
            .find_project("Work")
            .expect("find canonical")
            .expect("canonical exists")
            .uuid;

        stale
            .add_task("Work", "edited while stale")
            .expect("add stale task");
        let stale_uuid = stale
            .find_project("Work")
            .expect("find stale")
            .expect("stale exists")
            .uuid;
        assert_ne!(stale_uuid, canonical_uuid);
        let mut request = stale.export_sync_request().expect("export stale request");
        request.checkpoint = canonical_response.checkpoint;
        for project in &mut request.projects {
            project.sync_dirty = false;
        }

        let response = server
            .apply_server_sync(&request)
            .expect("resolve clean alias and dirty task");
        assert!(
            response
                .projects
                .iter()
                .any(|project| project.uuid == canonical_uuid)
        );
        assert!(
            response
                .tasks
                .iter()
                .any(|task| task.project_uuid == canonical_uuid)
        );
        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &response,
            )
            .expect("apply canonical response");
        assert_eq!(
            stale
                .find_project("Work")
                .expect("find converged")
                .expect("converged exists")
                .uuid,
            canonical_uuid
        );
    }

    #[test]
    fn independently_created_same_name_projects_converge() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let second_dir = tempfile::tempdir().expect("second temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let second = Storage::open(second_dir.path().join("tasks.db")).expect("open second");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        first.add_task("Work", "first").expect("add first");
        second.add_task("work", "second").expect("add second");
        let first_response = server
            .apply_server_sync(&first.export_sync_request().expect("export first"))
            .expect("sync first");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &first_response,
            )
            .expect("apply first");
        let second_response = server
            .apply_server_sync(&second.export_sync_request().expect("export second"))
            .expect("sync second");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &second_response,
            )
            .expect("apply second");

        let server_uuid = server
            .find_project("Work")
            .expect("find server Work")
            .expect("server Work exists")
            .uuid;
        assert_eq!(
            second
                .find_project("Work")
                .expect("find second Work")
                .expect("second Work exists")
                .uuid,
            server_uuid
        );
        assert_eq!(server.sync_tasks().expect("server tasks").len(), 2);
    }

    #[test]
    fn absent_clean_project_is_materialized() {
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let project = client.ensure_project("Recovered").expect("create project");
        let mut request = client.export_sync_request().expect("export request");
        for row in &mut request.projects {
            row.sync_dirty = false;
        }

        let response = server
            .apply_server_sync(&request)
            .expect("restore clean project");
        assert_eq!(
            server
                .find_project("Recovered")
                .expect("find restored")
                .expect("restored exists")
                .uuid,
            project.uuid
        );
        assert!(response.projects.iter().any(|row| row.uuid == project.uuid));
    }

    #[test]
    fn ambiguous_known_project_rename_is_a_conflict_and_rolls_back() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        client.ensure_project("Alpha").expect("create Alpha");
        client.ensure_project("Beta").expect("create Beta");
        let initial = server
            .apply_server_sync(&client.export_sync_request().expect("export initial"))
            .expect("sync initial");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial,
            )
            .expect("apply initial");

        let mut request = client.export_sync_request().expect("export rename");
        let alpha_uuid = request
            .projects
            .iter()
            .find(|project| project.name_normalized == "alpha")
            .expect("Alpha row")
            .uuid
            .clone();
        let beta_uuid = request
            .projects
            .iter()
            .find(|project| project.name_normalized == "beta")
            .expect("Beta row")
            .uuid
            .clone();
        let alpha = request
            .projects
            .iter_mut()
            .find(|project| project.uuid == alpha_uuid)
            .expect("mutable Alpha");
        alpha.name = "Beta".to_string();
        alpha.name_normalized = "beta".to_string();
        alpha.sync_dirty = true;
        let beta = request
            .projects
            .iter_mut()
            .find(|project| project.uuid == beta_uuid)
            .expect("mutable Beta");
        beta.name = "Gamma".to_string();
        beta.name_normalized = "gamma".to_string();
        beta.sync_dirty = true;

        let error = server.apply_server_sync(&request).unwrap_err();
        assert!(matches!(error, AppError::SyncConflict(_)));
        assert!(error.to_string().contains(&alpha_uuid));
        assert!(error.to_string().contains(&beta_uuid));
        assert!(server.find_project("Alpha").expect("find Alpha").is_some());
        assert!(server.find_project("Beta").expect("find Beta").is_some());
        assert!(server.find_project("Gamma").expect("find Gamma").is_none());
    }

    #[test]
    fn project_deletion_tombstones_children_and_blocks_stale_edits() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let stale_dir = tempfile::tempdir().expect("stale temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let stale = Storage::open(stale_dir.path().join("tasks.db")).expect("open stale");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        let task = first.add_task("Work", "child").expect("add child");
        let initial = server
            .apply_server_sync(&first.export_sync_request().expect("export initial"))
            .expect("sync initial");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &initial,
            )
            .expect("apply first");
        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &initial,
            )
            .expect("apply stale");
        let project_uuid = first
            .find_project("Work")
            .expect("find Work")
            .expect("Work exists")
            .uuid;

        let mut deletion = first.export_sync_request().expect("export deletion");
        deletion.projects.retain(|row| row.uuid != project_uuid);
        deletion.tasks.retain(|row| row.uuid != task.uuid);
        deletion.tombstones.push(SyncTombstone {
            entity_type: SyncEntityKind::Project,
            uuid: project_uuid.clone(),
            deleted_at: now_timestamp(),
            sync_revision: 0,
            sync_dirty: true,
        });
        let deleted = server.apply_server_sync(&deletion).expect("delete project");
        let project_tombstone = deleted
            .tombstones
            .iter()
            .find(|row| row.entity_type == SyncEntityKind::Project && row.uuid == project_uuid)
            .expect("project tombstone");
        let task_tombstone = deleted
            .tombstones
            .iter()
            .find(|row| row.entity_type == SyncEntityKind::Task && row.uuid == task.uuid)
            .expect("task tombstone");
        assert_eq!(
            task_tombstone.sync_revision,
            project_tombstone.sync_revision
        );

        let stale_id = stale
            .get_task_by_uuid(&task.uuid)
            .expect("get stale child")
            .expect("stale child exists")
            .id;
        stale.mark_done(stale_id).expect("edit stale child");
        let mut stale_request = stale.export_sync_request().expect("export stale edit");
        stale_request.checkpoint = deleted.checkpoint;
        let blocked = server
            .apply_server_sync(&stale_request)
            .expect("block stale child");
        assert!(
            blocked
                .tombstones
                .iter()
                .any(|row| row.entity_type == SyncEntityKind::Task && row.uuid == task.uuid)
        );
        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &blocked,
            )
            .expect("apply blocking tombstone");
        assert!(
            stale
                .get_task_by_uuid(&task.uuid)
                .expect("get child")
                .is_none()
        );
        assert!(server.sync_tasks().expect("server tasks").is_empty());
    }

    #[test]
    fn moving_existing_task_into_deleted_project_tombstones_server_task() {
        let deleting_dir = tempfile::tempdir().expect("deleting client temp dir");
        let stale_dir = tempfile::tempdir().expect("stale client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let deleting =
            Storage::open(deleting_dir.path().join("tasks.db")).expect("open deleting client");
        let stale = Storage::open(stale_dir.path().join("tasks.db")).expect("open stale client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");

        let task = deleting
            .add_task("Source", "move while stale")
            .expect("add task");
        deleting
            .ensure_project("Destination")
            .expect("create destination");
        let initial = server
            .apply_server_sync(&deleting.export_sync_request().expect("export initial"))
            .expect("sync initial");
        deleting
            .apply_sync_response(
                &deleting
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial,
            )
            .expect("apply initial");
        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &initial,
            )
            .expect("seed stale client");

        let destination_uuid = deleting
            .find_project("Destination")
            .expect("find destination")
            .expect("destination exists")
            .uuid;
        let mut deletion = deleting
            .export_sync_request()
            .expect("export project deletion");
        deletion
            .projects
            .retain(|project| project.uuid != destination_uuid);
        deletion.tombstones.push(SyncTombstone {
            entity_type: SyncEntityKind::Project,
            uuid: destination_uuid.clone(),
            deleted_at: now_timestamp(),
            sync_revision: 0,
            sync_dirty: true,
        });
        server
            .apply_server_sync(&deletion)
            .expect("delete destination on server");

        let stale_task_id = stale
            .get_task_by_uuid(&task.uuid)
            .expect("get stale task")
            .expect("stale task exists")
            .id;
        stale
            .move_task(stale_task_id, "Destination")
            .expect("move stale task");
        let rejected_move = server
            .apply_server_sync(&stale.export_sync_request().expect("export stale move"))
            .expect("reject move into deleted project");

        assert!(rejected_move.tombstones.iter().any(|tombstone| {
            tombstone.entity_type == SyncEntityKind::Project && tombstone.uuid == destination_uuid
        }));
        assert!(rejected_move.tombstones.iter().any(|tombstone| {
            tombstone.entity_type == SyncEntityKind::Task && tombstone.uuid == task.uuid
        }));
        assert!(server.sync_tasks().expect("server tasks").is_empty());

        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &rejected_move,
            )
            .expect("apply rejected move response");
        assert!(
            stale
                .get_task_by_uuid(&task.uuid)
                .expect("get reconciled task")
                .is_none()
        );

        let follow_up = server
            .apply_server_sync(&stale.export_sync_request().expect("export follow-up"))
            .expect("sync reconciled client");
        stale
            .apply_sync_response(
                &stale.export_sync_request().expect("capture local snapshot"),
                &follow_up,
            )
            .expect("apply follow-up");
        assert_eq!(
            server.sync_tasks().expect("server tasks"),
            stale.sync_tasks().expect("stale tasks")
        );
    }

    #[test]
    fn stale_creation_under_deleted_project_is_blocked_by_old_project_tombstone() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        client.ensure_project("Work").expect("create Work");
        let initial = server
            .apply_server_sync(&client.export_sync_request().expect("export initial"))
            .expect("sync initial");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial,
            )
            .expect("apply initial");
        let project_uuid = client
            .find_project("Work")
            .expect("find Work")
            .expect("Work exists")
            .uuid;
        let mut deletion = client.export_sync_request().expect("export deletion");
        deletion.projects.retain(|row| row.uuid != project_uuid);
        deletion.tombstones.push(SyncTombstone {
            entity_type: SyncEntityKind::Project,
            uuid: project_uuid.clone(),
            deleted_at: now_timestamp(),
            sync_revision: 0,
            sync_dirty: true,
        });
        let deleted = server.apply_server_sync(&deletion).expect("delete project");

        let mut stale = client.export_sync_request().expect("export stale snapshot");
        stale.checkpoint = deleted.checkpoint;
        stale.tasks.push(SyncTask {
            uuid: Uuid::new_v4().to_string(),
            project_uuid: project_uuid.clone(),
            sort_order: 1,
            title: "new stale child".to_string(),
            notes: String::new(),
            status: "pending".to_string(),
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
            completed_at: None,
            sync_revision: 0,
            sync_dirty: true,
        });
        let response = server
            .apply_server_sync(&stale)
            .expect("block stale creation");
        assert!(
            response.tombstones.iter().any(|row| {
                row.entity_type == SyncEntityKind::Project && row.uuid == project_uuid
            })
        );
        assert!(server.sync_tasks().expect("server tasks").is_empty());
    }

    #[test]
    fn orphan_task_conflict_is_actionable_and_transactional() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        client.ensure_project("Work").expect("create Work");
        let initial = server
            .apply_server_sync(&client.export_sync_request().expect("export initial"))
            .expect("sync initial");
        client
            .apply_sync_response(
                &client
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial,
            )
            .expect("apply initial");
        let mut request = client.export_sync_request().expect("export conflict");
        let work = request
            .projects
            .iter_mut()
            .find(|row| row.name_normalized == "work")
            .expect("Work row");
        work.name = "Renamed".to_string();
        work.name_normalized = "renamed".to_string();
        work.sync_dirty = true;
        let task_uuid = Uuid::new_v4().to_string();
        request.tasks.push(SyncTask {
            uuid: task_uuid.clone(),
            project_uuid: "missing-project".to_string(),
            sort_order: 1,
            title: "orphan".to_string(),
            notes: String::new(),
            status: "pending".to_string(),
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
            completed_at: None,
            sync_revision: 0,
            sync_dirty: true,
        });

        let error = server.apply_server_sync(&request).unwrap_err();
        assert!(matches!(error, AppError::SyncConflict(_)));
        let message = error.to_string();
        assert!(message.contains(&task_uuid));
        assert!(message.contains("ct update"));
        assert!(message.contains("ct delete"));
        assert!(server.find_project("Work").expect("find Work").is_some());
        assert!(
            server
                .find_project("Renamed")
                .expect("find Renamed")
                .is_none()
        );
    }

    #[test]
    fn concurrent_same_position_additions_converge_to_contiguous_uuid_order() {
        let first_dir = tempfile::tempdir().expect("first temp dir");
        let second_dir = tempfile::tempdir().expect("second temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let first = Storage::open(first_dir.path().join("tasks.db")).expect("open first");
        let second = Storage::open(second_dir.path().join("tasks.db")).expect("open second");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        first.add_task("Work", "base").expect("add base");
        let initial = server
            .apply_server_sync(&first.export_sync_request().expect("export initial"))
            .expect("sync initial");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &initial,
            )
            .expect("apply first");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &initial,
            )
            .expect("apply second");
        first
            .add_task("Work", "from first")
            .expect("add first concurrent");
        second
            .add_task("Work", "from second")
            .expect("add second concurrent");

        let first_add = server
            .apply_server_sync(&first.export_sync_request().expect("export first add"))
            .expect("sync first add");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &first_add,
            )
            .expect("apply first add");
        let second_add = server
            .apply_server_sync(&second.export_sync_request().expect("export second add"))
            .expect("sync second add");
        second
            .apply_sync_response(
                &second
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &second_add,
            )
            .expect("apply second add");
        let final_first = server
            .apply_server_sync(&first.export_sync_request().expect("export final first"))
            .expect("sync final first");
        first
            .apply_sync_response(
                &first.export_sync_request().expect("capture local snapshot"),
                &final_first,
            )
            .expect("apply final first");

        let server_tasks = server.sync_tasks().expect("server tasks");
        let first_tasks = first.sync_tasks().expect("first tasks");
        let second_tasks = second.sync_tasks().expect("second tasks");
        assert_eq!(first_tasks, server_tasks);
        assert_eq!(second_tasks, server_tasks);
        assert_eq!(
            server_tasks
                .iter()
                .map(|task| task.sort_order)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let tied = &server_tasks[1..];
        assert!(tied[0].uuid < tied[1].uuid);
    }

    #[test]
    fn retries_are_idempotent_and_checkpoint_ahead_conflicts() {
        let client_dir = tempfile::tempdir().expect("client temp dir");
        let server_dir = tempfile::tempdir().expect("server temp dir");
        let client = Storage::open(client_dir.path().join("tasks.db")).expect("open client");
        let server = Storage::open(server_dir.path().join("tasks.db")).expect("open server");
        client.add_task("Work", "retry me").expect("add task");
        let request = client.export_sync_request().expect("export request");
        let first = server.apply_server_sync(&request).expect("first sync");
        let retry = server.apply_server_sync(&request).expect("retry sync");
        assert_eq!(retry.checkpoint, first.checkpoint);

        let mut ahead = request;
        ahead.checkpoint = first.checkpoint + 1;
        let error = server.apply_server_sync(&ahead).unwrap_err();
        assert!(matches!(error, AppError::SyncConflict(_)));
        assert!(error.to_string().contains("ahead"));
        assert_eq!(
            server.max_sync_revision().expect("server revision"),
            first.checkpoint
        );
    }

    #[test]
    fn malformed_snapshots_are_bad_requests() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");
        let mut negative = storage.export_sync_request().expect("export negative");
        negative.checkpoint = -1;
        assert!(matches!(
            storage.apply_server_sync(&negative),
            Err(AppError::SyncBadRequest(_))
        ));

        let mut invalid_status = storage.export_sync_request().expect("export status");
        invalid_status.tasks.push(SyncTask {
            uuid: Uuid::new_v4().to_string(),
            project_uuid: invalid_status.projects[0].uuid.clone(),
            sort_order: 1,
            title: "invalid".to_string(),
            notes: String::new(),
            status: "unknown".to_string(),
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
            completed_at: None,
            sync_revision: 0,
            sync_dirty: true,
        });
        assert!(matches!(
            storage.apply_server_sync(&invalid_status),
            Err(AppError::SyncBadRequest(_))
        ));

        let mut duplicate_name = storage.export_sync_request().expect("export duplicate");
        let mut duplicate = duplicate_name.projects[0].clone();
        duplicate.uuid = Uuid::new_v4().to_string();
        duplicate_name.projects.push(duplicate);
        assert!(matches!(
            storage.apply_server_sync(&duplicate_name),
            Err(AppError::SyncBadRequest(_))
        ));

        let mut invalid_position = storage.export_sync_request().expect("export position");
        invalid_position.tasks.push(SyncTask {
            uuid: Uuid::new_v4().to_string(),
            project_uuid: invalid_position.projects[0].uuid.clone(),
            sort_order: 0,
            title: "invalid position".to_string(),
            notes: String::new(),
            status: "pending".to_string(),
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
            completed_at: None,
            sync_revision: 0,
            sync_dirty: true,
        });
        assert!(matches!(
            storage.apply_server_sync(&invalid_position),
            Err(AppError::SyncBadRequest(_))
        ));
    }

    #[test]
    fn sync_response_rolls_back_when_apply_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");
        let now = now_timestamp();
        let response = SyncResponse {
            checkpoint: 2,
            projects: vec![SyncProject {
                uuid: Uuid::new_v4().to_string(),
                name: "Remote".to_string(),
                name_normalized: "remote".to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
                archived_at: None,
                sync_revision: 1,
                sync_dirty: false,
            }],
            tasks: vec![SyncTask {
                uuid: Uuid::new_v4().to_string(),
                project_uuid: "missing-project".to_string(),
                sort_order: 1,
                title: "orphan".to_string(),
                notes: String::new(),
                status: TaskStatus::Pending.as_str().to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
                completed_at: None,
                sync_revision: 2,
                sync_dirty: false,
            }],
            tombstones: Vec::new(),
        };

        assert!(
            storage
                .apply_sync_response(
                    &storage
                        .export_sync_request()
                        .expect("capture local snapshot"),
                    &response
                )
                .is_err()
        );
        assert!(
            storage
                .find_project("Remote")
                .expect("find remote")
                .is_none()
        );
        assert_eq!(
            storage
                .export_sync_request()
                .expect("export after failed apply")
                .checkpoint,
            0
        );
    }

    #[test]
    fn project_tombstone_removes_referencing_tasks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");
        storage.add_task("Work", "write tests").expect("add task");
        let project_uuid = storage
            .find_project("Work")
            .expect("find project")
            .expect("project exists")
            .uuid;
        let response = SyncResponse {
            checkpoint: 1,
            projects: Vec::new(),
            tasks: Vec::new(),
            tombstones: vec![SyncTombstone {
                entity_type: SyncEntityKind::Project,
                uuid: project_uuid,
                deleted_at: now_timestamp(),
                sync_revision: 1,
                sync_dirty: false,
            }],
        };

        storage
            .apply_sync_response(
                &storage
                    .export_sync_request()
                    .expect("capture local snapshot"),
                &response,
            )
            .expect("apply project tombstone");
        assert!(storage.find_project("Work").expect("find Work").is_none());
        assert!(storage.sync_tasks().expect("tasks").is_empty());
    }
}
