//! Embedded native shell templates for `soli generate client`.

pub mod android;
pub mod android_fcm;
pub mod ios;
pub mod linux;
pub mod windows;

/// Shared substitution context.
pub struct ClientCtx {
    pub app_name: String,
    pub start_url: String,
    pub host: String,
    pub package_id: String,
    pub scheme: String,
    pub team_id: String,
}

impl ClientCtx {
    pub fn apply(&self, template: &str) -> String {
        template
            .replace("{{APP_NAME}}", &self.app_name)
            .replace("{{START_URL}}", &self.start_url)
            .replace("{{HOST}}", &self.host)
            .replace("{{PACKAGE_ID}}", &self.package_id)
            .replace("{{SCHEME}}", &self.scheme)
            .replace("{{TEAM_ID}}", &self.team_id)
            .replace("{{PACKAGE_PATH}}", &self.package_id.replace('.', "/"))
            .replace("{{JAVA_PACKAGE}}", &self.package_id)
    }
}
