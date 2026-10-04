//! The iOS over-the-air install manifest.
//!
//! Safari opens `itms-services://?action=download-manifest&url=<manifest>`,
//! fetches this plist itself, then fetches the IPA the plist names. Both URLs
//! must be HTTPS, and neither request carries the browser's cookies, which is
//! why the install links are public by secret rather than behind the gate.

use super::xml_escape;

pub struct ManifestInput<'a> {
    /// Absolute HTTPS URL of the IPA.
    pub ipa_url: &'a str,
    pub bundle_id: &'a str,
    pub version: &'a str,
    pub title: &'a str,
}

pub fn render(input: &ManifestInput<'_>) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>items</key>
  <array>
    <dict>
      <key>assets</key>
      <array>
        <dict>
          <key>kind</key>
          <string>software-package</string>
          <key>url</key>
          <string>{url}</string>
        </dict>
      </array>
      <key>metadata</key>
      <dict>
        <key>bundle-identifier</key>
        <string>{bundle}</string>
        <key>bundle-version</key>
        <string>{version}</string>
        <key>kind</key>
        <string>software</string>
        <key>title</key>
        <string>{title}</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>
"#,
        url = xml_escape(input.ipa_url),
        bundle = xml_escape(input.bundle_id),
        version = xml_escape(input.version),
        title = xml_escape(input.title),
    )
}

/// The link Safari follows to start the install.
pub fn itms_services_link(manifest_url: &str) -> String {
    format!(
        "itms-services://?action=download-manifest&url={}",
        urlencoding::encode(manifest_url)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_names_the_ipa_and_escapes_values() {
        let out = render(&ManifestInput {
            ipa_url: "https://a.test/__soli/mobile/i/tok/download?x=1&y=2",
            bundle_id: "com.example.shop",
            version: "1.4.0",
            title: "Shop <beta>",
        });
        assert!(out.contains("<string>software-package</string>"));
        assert!(out.contains("download?x=1&amp;y=2"));
        assert!(out.contains("<string>com.example.shop</string>"));
        assert!(out.contains("Shop &lt;beta&gt;"));
    }

    #[test]
    fn itms_link_encodes_the_manifest_url() {
        assert_eq!(
            itms_services_link("https://a.test/m.plist"),
            "itms-services://?action=download-manifest&url=https%3A%2F%2Fa.test%2Fm.plist"
        );
    }
}
