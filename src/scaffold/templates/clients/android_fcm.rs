//! Gradle + Firebase Android shell templates.

pub const README: &str = include_str!("android_fcm/README.txt");
pub const SETTINGS_GRADLE: &str = include_str!("android_fcm/settings.gradle");
pub const ROOT_BUILD_GRADLE: &str = include_str!("android_fcm/build.gradle");
pub const GRADLE_PROPERTIES: &str = include_str!("android_fcm/gradle.properties");
pub const APP_BUILD_GRADLE: &str = include_str!("android_fcm/app_build.gradle");
pub const MANIFEST: &str = include_str!("android_fcm/AndroidManifest.xml");
pub const MAIN_ACTIVITY: &str = include_str!("android_fcm/MainActivity.java");
pub const FCM_SERVICE: &str = include_str!("android_fcm/SoliFirebaseMessagingService.java");
pub const GOOGLE_SERVICES_PLACEHOLDER: &str =
    include_str!("android_fcm/google-services.json.example");
