use actix_web::{
    error::*,
    web::{self, block, Data, Json},
    HttpResponse, Responder,
};
use chrono::{Duration, Local};
use dashmap::DashMap;
use serde_derive::Deserialize;

use crate::{database::Database, error::TimeError, models::UserId, requests::*};

pub type HeartBeatMemoryStore = DashMap<i32, (HeartBeat, chrono::NaiveDateTime, chrono::Duration)>;

#[derive(Deserialize)]
pub struct RenameRequest {
    from: String,
    to: String,
}

#[post("/update")]
pub async fn update(
    user: UserId,
    heartbeat: Json<HeartBeat>,
    db: Data<Database>,
    heartbeats: Data<HeartBeatMemoryStore>,
) -> Result<impl Responder, TimeError> {
    if let Some(project) = &heartbeat.project_name {
        if project.len() > 64 {
            return Err(TimeError::InvalidLength(
                "Project name is over 64 chars".to_string(),
            ));
        }
    }
    if let Some(language) = &heartbeat.language {
        if language.len() > 32 {
            return Err(TimeError::InvalidLength(
                "Language is over 32 chars".to_string(),
            ));
        }
    }
    if let Some(editor) = &heartbeat.editor_name {
        if editor.len() > 32 {
            return Err(TimeError::InvalidLength(
                "Editor name is over 32 chars".to_string(),
            ));
        }
    }
    if let Some(hostname) = &heartbeat.hostname {
        if hostname.len() > 32 {
            return Err(TimeError::InvalidLength(
                "Hostname is over 32 chars".to_string(),
            ));
        }
    }
    match heartbeats.get(&user.id) {
        Some(activity) => {
            let (current_heartbeat, start, mut duration) = activity.to_owned();
            drop(activity);
            let curtime = Local::now().naive_local();
            if heartbeat.eq(&current_heartbeat) {
                if curtime.signed_duration_since(start + duration) > Duration::seconds(900) {
                    // If the user sends a heartbeat but maximum activity duration has been exceeded,
                    // end session and start new
                    db.get()?
                        .add_activity(user.id, current_heartbeat, start, duration)
                        .map_err(ErrorInternalServerError)?;

                    heartbeats.insert(
                        user.id,
                        (
                            heartbeat.into_inner(),
                            Local::now().naive_local(),
                            Duration::seconds(0),
                        ),
                    );
                    Ok(HttpResponse::Ok().body(0i32.to_string()))
                } else {
                    // Extend current coding session if heartbeat matches and it has been under the maximum duration of a break
                    heartbeats.insert(
                        user.id,
                        (
                            heartbeat.into_inner(),
                            start,
                            curtime.signed_duration_since(start),
                        ),
                    );
                    Ok(HttpResponse::Ok().body(curtime.signed_duration_since(start).to_string()))
                }
            } else {
                // Flush current session and start new session if heartbeat changes
                if curtime.signed_duration_since(start + duration) < Duration::seconds(30) {
                    duration = curtime.signed_duration_since(start);
                }

                db.get()?
                    .add_activity(user.id, current_heartbeat, start, duration)
                    .map_err(ErrorInternalServerError)?;

                heartbeats.insert(
                    user.id,
                    (
                        heartbeat.into_inner(),
                        Local::now().naive_local(),
                        Duration::seconds(0),
                    ),
                );
                Ok(HttpResponse::Ok().body(0i32.to_string()))
            }
        }
        None => {
            // If the user has not sent a heartbeat during this session
            heartbeats.insert(
                user.id,
                (
                    heartbeat.into_inner(),
                    Local::now().naive_local(),
                    Duration::seconds(0),
                ),
            );
            Ok(HttpResponse::Ok().body(0.to_string()))
        }
    }
}

#[post("/flush")]
pub async fn flush(
    user: UserId,
    db: Data<Database>,
    heartbeats: Data<HeartBeatMemoryStore>,
) -> Result<impl Responder, TimeError> {
    if let Some(heartbeat) = heartbeats.get(&user.id) {
        let (inner_heartbeat, start, duration) = heartbeat.to_owned();
        drop(heartbeat);
        heartbeats.remove(&user.id);
        block(move || {
            db.get()?
                .add_activity(user.id, inner_heartbeat, start, duration)
        })
        .await??;
    }
    Ok(HttpResponse::Ok().finish())
}

#[delete("/delete")]
pub async fn delete(
    user: UserId,
    db: Data<Database>,
    body: String,
) -> Result<impl Responder, TimeError> {
    let deleted = block(move || {
        db.get()?
            .delete_activity(user.id, body.parse::<i32>().map_err(ErrorBadRequest)?)
    })
    .await??;
    if deleted {
        Ok(HttpResponse::Ok().finish())
    } else {
        Err(TimeError::BadId)
    }
}

#[post("/rename")]
pub async fn rename_project(
    user: UserId,
    db: Data<Database>,
    body: Json<RenameRequest>,
) -> Result<impl Responder, TimeError> {
    let renamed = block(move || db.get()?.rename_project(user.id, &body.from, &body.to)).await??;

    Ok(web::Json(json!({ "affected_activities": renamed })))
}
