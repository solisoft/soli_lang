//! XMP metadata packet construction for Factur-X / PDF-A-3b.
//!
//! The packet declares PDF/A-3b conformance (`pdfaid:part=3`,
//! `pdfaid:conformance=B`), basic Dublin Core / PDF / XMP fields, the **Factur-X
//! extension schema** description block (required so validators accept the
//! custom `fx:` namespace), and the `fx:` values themselves. Validators are
//! byte-fussy here, so the structure follows the PDFlib/akretion reference.

use time::OffsetDateTime;

use super::{FacturxMetadata, Profile};

/// XML-escape text for inclusion in the packet.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format an `OffsetDateTime` as an XMP timestamp (`2025-11-28T00:00:00+00:00`).
fn xmp_date(dt: OffsetDateTime) -> String {
    let o = dt.offset();
    let (oh, om, _) = o.as_hms();
    let sign = if oh < 0 || om < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        sign,
        oh.unsigned_abs(),
        om.unsigned_abs(),
    )
}

/// Build the complete XMP packet for the given profile + metadata.
pub fn build(profile: Profile, meta: &FacturxMetadata) -> String {
    let ts = xmp_date(meta.created);
    let level = profile.xmp_level();
    format!(
        r#"<?xpacket begin="{bom}" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" rdf:about="">
      <pdfaid:part>3</pdfaid:part>
      <pdfaid:conformance>B</pdfaid:conformance>
    </rdf:Description>
    <rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" rdf:about="">
      <dc:title><rdf:Alt><rdf:li xml:lang="x-default">{title}</rdf:li></rdf:Alt></dc:title>
      <dc:creator><rdf:Seq><rdf:li>{author}</rdf:li></rdf:Seq></dc:creator>
      <dc:description><rdf:Alt><rdf:li xml:lang="x-default">{subject}</rdf:li></rdf:Alt></dc:description>
    </rdf:Description>
    <rdf:Description xmlns:pdf="http://ns.adobe.com/pdf/1.3/" rdf:about="">
      <pdf:Producer>{producer}</pdf:Producer>
    </rdf:Description>
    <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" rdf:about="">
      <xmp:CreatorTool>{creator_tool}</xmp:CreatorTool>
      <xmp:CreateDate>{ts}</xmp:CreateDate>
      <xmp:ModifyDate>{ts}</xmp:ModifyDate>
    </rdf:Description>
    <rdf:Description xmlns:pdfaExtension="http://www.aiim.org/pdfa/ns/extension/" xmlns:pdfaSchema="http://www.aiim.org/pdfa/ns/schema#" xmlns:pdfaProperty="http://www.aiim.org/pdfa/ns/property#" rdf:about="">
      <pdfaExtension:schemas>
        <rdf:Bag>
          <rdf:li rdf:parseType="Resource">
            <pdfaSchema:schema>Factur-X PDFA Extension Schema</pdfaSchema:schema>
            <pdfaSchema:namespaceURI>urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#</pdfaSchema:namespaceURI>
            <pdfaSchema:prefix>fx</pdfaSchema:prefix>
            <pdfaSchema:property>
              <rdf:Seq>
                <rdf:li rdf:parseType="Resource">
                  <pdfaProperty:name>DocumentFileName</pdfaProperty:name>
                  <pdfaProperty:valueType>Text</pdfaProperty:valueType>
                  <pdfaProperty:category>external</pdfaProperty:category>
                  <pdfaProperty:description>The name of the embedded XML document</pdfaProperty:description>
                </rdf:li>
                <rdf:li rdf:parseType="Resource">
                  <pdfaProperty:name>DocumentType</pdfaProperty:name>
                  <pdfaProperty:valueType>Text</pdfaProperty:valueType>
                  <pdfaProperty:category>external</pdfaProperty:category>
                  <pdfaProperty:description>The type of the hybrid document in capital letters, e.g. INVOICE or ORDER</pdfaProperty:description>
                </rdf:li>
                <rdf:li rdf:parseType="Resource">
                  <pdfaProperty:name>Version</pdfaProperty:name>
                  <pdfaProperty:valueType>Text</pdfaProperty:valueType>
                  <pdfaProperty:category>external</pdfaProperty:category>
                  <pdfaProperty:description>The actual version of the standard applying to the embedded XML document</pdfaProperty:description>
                </rdf:li>
                <rdf:li rdf:parseType="Resource">
                  <pdfaProperty:name>ConformanceLevel</pdfaProperty:name>
                  <pdfaProperty:valueType>Text</pdfaProperty:valueType>
                  <pdfaProperty:category>external</pdfaProperty:category>
                  <pdfaProperty:description>The conformance level of the embedded XML document</pdfaProperty:description>
                </rdf:li>
              </rdf:Seq>
            </pdfaSchema:property>
          </rdf:li>
        </rdf:Bag>
      </pdfaExtension:schemas>
    </rdf:Description>
    <rdf:Description xmlns:fx="urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#" rdf:about="">
      <fx:DocumentType>INVOICE</fx:DocumentType>
      <fx:DocumentFileName>factur-x.xml</fx:DocumentFileName>
      <fx:Version>1.0</fx:Version>
      <fx:ConformanceLevel>{level}</fx:ConformanceLevel>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#,
        bom = '\u{feff}',
        title = esc(&meta.title),
        author = esc(&meta.author),
        subject = esc(&meta.subject),
        producer = esc(&meta.producer),
        creator_tool = esc(&meta.creator_tool),
        ts = ts,
        level = level,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_has_required_markers() {
        let meta = FacturxMetadata::default();
        let xmp = build(Profile::En16931, &meta);
        assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
        assert!(xmp.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
        assert!(xmp.contains("urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"));
        assert!(xmp.contains("<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"));
        assert!(xmp.contains("<fx:DocumentFileName>factur-x.xml</fx:DocumentFileName>"));
        assert!(xmp.starts_with("<?xpacket"));
    }
}
