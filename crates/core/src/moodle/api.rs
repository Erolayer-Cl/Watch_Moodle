use serde_json::json;

use super::Client;
use super::types::*;
use crate::Result;

impl Client {
    pub async fn site_info(&self) -> Result<SiteInfo> {
        self.call("core_webservice_get_site_info", json!({})).await
    }

    pub async fn user_courses(&self, userid: i64) -> Result<Vec<UserCourse>> {
        self.call("core_enrol_get_users_courses", json!({ "userid": userid })).await
    }

    /// Detalle de cursos; se usa por los `contacts` (profesores).
    pub async fn courses_by_ids(&self, ids: &[i64]) -> Result<Vec<CourseDetail>> {
        let value = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let res: CoursesByField =
            self.call("core_course_get_courses_by_field", json!({ "field": "ids", "value": value })).await?;
        Ok(res.courses)
    }

    pub async fn course_contents(&self, courseid: i64) -> Result<Vec<Section>> {
        self.call("core_course_get_contents", json!({ "courseid": courseid })).await
    }

    pub async fn forums(&self, courseids: &[i64]) -> Result<Vec<Forum>> {
        self.call("mod_forum_get_forums_by_courses", json!({ "courseids": courseids })).await
    }

    /// Discusiones más recientes de un foro. `legacy` usa la función antigua
    /// (Moodle < 3.7).
    pub async fn discussions(&self, forumid: i64, per_page: u32, legacy: bool) -> Result<Vec<Discussion>> {
        let res: Discussions = if legacy {
            self.call(
                "mod_forum_get_forum_discussions_paginated",
                json!({ "forumid": forumid, "sortby": "timemodified", "sortdirection": "DESC", "page": 0, "perpage": per_page }),
            )
            .await?
        } else {
            self.call("mod_forum_get_forum_discussions", json!({ "forumid": forumid, "page": 0, "perpage": per_page }))
                .await?
        };
        Ok(res.discussions)
    }

    pub async fn assignments(&self, courseids: &[i64]) -> Result<Vec<AssignCourse>> {
        let res: AssignmentsResponse =
            self.call("mod_assign_get_assignments", json!({ "courseids": courseids })).await?;
        Ok(res.courses)
    }

    pub async fn submission_status(&self, assignid: i64) -> Result<SubmissionStatus> {
        self.call("mod_assign_get_submission_status", json!({ "assignid": assignid })).await
    }
}
