//! ProtoFile（项目级 proto 文件）仓储：批量保存 / 列表 / 删除。
//!
//! 保存走「整体替换」语义：编辑器每次提交全部文件集（含新增/修改），
//! 删除的文件由命令层先调 [`delete_proto_file`]（或恢复时清空后批量写入）。

use crate::db::db_err;
use sqlx::QueryBuilder;
use uuid::Uuid;

use miyo_core::model::ProtoFile;
use miyo_core::Result;

/// 列出项目的全部 proto 文件（按名称排序，列表稳定）。
pub async fn list_proto_files(db: &sqlx::SqlitePool, project_id: Uuid) -> Result<Vec<ProtoFile>> {
    let rows: Vec<ProtoFileRow> = sqlx::query_as(
        "SELECT id, project_id, name, content, created_at, updated_at
         FROM proto_files WHERE project_id = ? ORDER BY name",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(ProtoFileRow::into_model).collect()
}

/// 删除单条 proto 文件。
pub async fn delete_proto_file(db: &sqlx::SqlitePool, proto_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM proto_files WHERE id = ?")
        .bind(proto_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// 批量写入 proto 文件（编辑器保存 / 备份恢复用）：事务外调用方控制事务，
/// 每批 200 行多值 INSERT。
pub async fn save_proto_files_bulk(
    conn: &mut sqlx::SqliteConnection,
    files: &[ProtoFile],
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let rows: Vec<ProtoFileRow> = files
        .iter()
        .map(ProtoFileRow::from_model)
        .collect::<Result<Vec<_>>>()?;
    for chunk in rows.chunks(200) {
        let mut qb = QueryBuilder::new(
            "INSERT INTO proto_files (id, project_id, name, content, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.project_id)
                .push_bind(&row.name)
                .push_bind(&row.content)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.build().execute(&mut *conn).await.map_err(db_err)?;
    }
    Ok(())
}

/// 编辑器保存入口：替换式批量 upsert（同 id 覆盖，新 id 插入）。
pub async fn save_proto_files(db: &sqlx::SqlitePool, files: &[ProtoFile]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let rows: Vec<ProtoFileRow> = files
        .iter()
        .map(ProtoFileRow::from_model)
        .collect::<Result<Vec<_>>>()?;
    for chunk in rows.chunks(200) {
        let mut qb = QueryBuilder::new(
            "INSERT INTO proto_files (id, project_id, name, content, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.project_id)
                .push_bind(&row.name)
                .push_bind(&row.content)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.push(" ON CONFLICT(id) DO UPDATE SET name = excluded.name, content = excluded.content, updated_at = excluded.updated_at");
        qb.build().execute(db).await.map_err(db_err)?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ProtoFileRow {
    id: String,
    project_id: String,
    name: String,
    content: String,
    created_at: String,
    updated_at: String,
}

impl ProtoFileRow {
    fn from_model(file: &ProtoFile) -> Result<Self> {
        Ok(Self {
            id: file.id.to_string(),
            project_id: file.project_id.to_string(),
            name: file.name.clone(),
            content: file.content.clone(),
            created_at: file.created_at.to_rfc3339(),
            updated_at: file.updated_at.to_rfc3339(),
        })
    }

    fn into_model(self) -> Result<ProtoFile> {
        Ok(ProtoFile {
            id: super::rows::parse_uuid(&self.id)?,
            project_id: super::rows::parse_uuid(&self.project_id)?,
            name: self.name,
            content: self.content,
            created_at: super::rows::parse_time(&self.created_at)?,
            updated_at: super::rows::parse_time(&self.updated_at)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository as repo;
    use chrono::Utc;
    use miyo_core::model::Project;

    fn mk_file(project_id: Uuid, name: &str) -> ProtoFile {
        ProtoFile {
            id: Uuid::new_v4(),
            project_id,
            name: name.into(),
            content: format!("syntax = \"proto3\";\n// {name}"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn save_list_upsert_roundtrip() {
        let path = std::env::temp_dir().join(format!("miyo-proto-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = crate::db::init_db(&path).await.expect("建库");
        let project_id = Uuid::new_v4();
        // proto_files.project_id 外键引用 projects：先落库项目（FK 开启）
        repo::save_project(
            &db,
            &Project {
                id: project_id,
                name: "proto 测试项目".into(),
                description: String::new(),
                variables: Default::default(),
                team_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("落库项目");

        // 空列表不写不报错
        save_proto_files(&db, &[]).await.expect("空集");

        let mut files = vec![
            mk_file(project_id, "b.proto"),
            mk_file(project_id, "a.proto"),
        ];
        save_proto_files(&db, &files).await.expect("批量写入");
        let listed = list_proto_files(&db, project_id).await.expect("列表");
        assert_eq!(listed.len(), 2, "两条都写入");
        // 按名称排序
        assert_eq!(listed[0].name, "a.proto");
        assert_eq!(listed[1].name, "b.proto");

        // upsert：同 id 修改内容，另加一条新文件
        files[0].content = "syntax = \"proto3\";\n// updated".into();
        files.push(mk_file(project_id, "c.proto"));
        save_proto_files(&db, &files).await.expect("upsert");
        let listed = list_proto_files(&db, project_id).await.expect("列表");
        assert_eq!(listed.len(), 3, "新文件插入，旧文件覆盖");
        assert_eq!(listed[1].content, "syntax = \"proto3\";\n// updated");

        // 删除一条
        delete_proto_file(&db, files[0].id).await.expect("删除");
        let listed = list_proto_files(&db, project_id).await.expect("列表");
        assert_eq!(listed.len(), 2);

        db.close().await;
        let _ = std::fs::remove_file(&path);
    }
}
