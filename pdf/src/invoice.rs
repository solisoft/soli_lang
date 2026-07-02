//! A typed invoice model that is the *single source of truth* for both halves
//! of the output: the visual PDF (via [`Invoice::to_render_data`], which maps
//! onto the template's `${...}` paths) and the embedded Factur-X CII XML (via
//! [`Invoice::to_cii_xml`]). Totals and the VAT breakdown are **computed** from
//! the line items, so the human-readable document and the machine-readable XML
//! can never disagree — which is the whole point of Factur-X.
//!
//! The generated CII mirrors the structure of the bundled, veraPDF-validated
//! `tests/fixtures/factur-x.xml` EN 16931 sample, generalised to any number of
//! lines and VAT rates.

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Value};

use crate::error::{PdfError, Result};
use crate::facturx::Profile;

/// A monetary amount stored as an exact number of hundredths (e.g. cents), so
/// arithmetic over many lines stays exact and formats to two decimals.
///
/// Deserialises from a JSON number (`100`, `100.5`) or a numeric string
/// (`"100.50"`); serialises back to the two-decimal string form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Amount(i64);

impl Amount {
    /// Build from a whole number of hundredths.
    pub fn from_hundredths(h: i64) -> Self {
        Amount(h)
    }

    /// Build from a decimal value, rounding to the nearest hundredth.
    pub fn from_f64(v: f64) -> Self {
        Amount((v * 100.0).round() as i64)
    }

    /// The amount as a whole number of hundredths.
    pub fn hundredths(self) -> i64 {
        self.0
    }

    /// Parse a decimal string (`"100"`, `"100.5"`, `"-12.34"`) into an amount.
    fn parse(s: &str) -> Option<Amount> {
        s.trim().parse::<f64>().ok().map(Amount::from_f64)
    }

    /// Format with exactly two decimals (`1234` hundredths → `"12.34"`), as CII
    /// monetary fields require.
    pub fn format(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        format!("{sign}{}.{:02}", abs / 100, abs % 100)
    }
}

impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.format())
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Amount, D::Error> {
        struct AmountVisitor;
        impl Visitor<'_> for AmountVisitor {
            type Value = Amount;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a monetary amount (number or numeric string)")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Amount, E> {
                Ok(Amount(v * 100))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Amount, E> {
                Ok(Amount((v as i64) * 100))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Amount, E> {
                Ok(Amount::from_f64(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Amount, E> {
                Amount::parse(v).ok_or_else(|| E::custom(format!("invalid amount: {v:?}")))
            }
        }
        d.deserialize_any(AmountVisitor)
    }
}

/// A trading party (seller or buyer). `country` is an ISO 3166-1 alpha-2 code
/// (e.g. `"FR"`) used in the CII; `country_name` is an optional human label for
/// the visual PDF (falls back to the code).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Party {
    pub name: String,
    #[serde(default)]
    pub address_line: String,
    #[serde(default)]
    pub postcode: String,
    #[serde(default)]
    pub city: String,
    /// ISO 3166-1 alpha-2 country code (BT-40 / BT-55).
    #[serde(default)]
    pub country: String,
    /// Optional display name for the country shown on the PDF.
    #[serde(default)]
    pub country_name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    /// VAT identifier — BT-31 (seller) / BT-48 (buyer). Required by EN 16931 on
    /// the seller when VAT is charged.
    #[serde(default)]
    pub vat_id: Option<String>,
    /// IBAN for payment — used to build the EPC "scan-to-pay" QR (seller only).
    #[serde(default)]
    pub iban: Option<String>,
    /// BIC/SWIFT (optional within the EEA for EPC version 002).
    #[serde(default)]
    pub bic: Option<String>,
}

impl Party {
    fn country_display(&self) -> &str {
        self.country_name.as_deref().unwrap_or(&self.country)
    }
}

/// One invoice line (BG-25). The net unit price times the quantity gives the
/// line total; VAT is applied per line and aggregated into the header breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    /// Item name (BT-153).
    pub name: String,
    /// Billed quantity (BT-129).
    #[serde(default = "one")]
    pub quantity: f64,
    /// Unit of measure code (BT-130). Defaults to `C62` ("one"/piece).
    #[serde(default = "default_unit")]
    pub unit_code: String,
    /// Net price of one unit (BT-146).
    pub unit_price: Amount,
    /// VAT rate as a percentage (BT-152), e.g. `20.0`.
    #[serde(default)]
    pub vat_rate: f64,
    /// VAT category code (BT-151). Defaults to `S` (standard rate).
    #[serde(default = "default_category")]
    pub vat_category: String,
}

fn one() -> f64 {
    1.0
}
fn default_unit() -> String {
    "C62".to_string()
}
fn default_category() -> String {
    "S".to_string()
}
fn default_type_code() -> String {
    "380".to_string()
}

impl Line {
    /// Net line total in hundredths: `round(unit_price * quantity)`.
    fn net_hundredths(&self) -> i64 {
        (self.unit_price.hundredths() as f64 * self.quantity).round() as i64
    }
}

/// A document-level allowance (discount, BG-20) or charge (fee/shipping,
/// BG-21). Exactly one of `amount` / `percent` must be set; a percent resolves
/// against the line-net total (BT-93 / BT-100 basis). The (category, rate)
/// pair adjusts that group's VAT basis, creating the group when no line uses it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowanceCharge {
    /// Reason text (BT-97 / BT-104).
    pub reason: String,
    /// Fixed amount (BT-92 / BT-99).
    #[serde(default)]
    pub amount: Option<Amount>,
    /// Percentage (BT-94 / BT-101) of the line-net total.
    #[serde(default)]
    pub percent: Option<f64>,
    /// VAT rate for this allowance/charge (BT-96 / BT-103).
    #[serde(default)]
    pub vat_rate: f64,
    /// VAT category code (BT-95 / BT-102). Defaults to `S`.
    #[serde(default = "default_category")]
    pub vat_category: String,
}

impl AllowanceCharge {
    /// Resolved amount in hundredths (percent entries resolve once, here).
    fn resolve_hundredths(&self, line_total: i64) -> i64 {
        match self.percent {
            Some(p) => (line_total as f64 * p / 100.0).round() as i64,
            None => self.amount.unwrap_or_default().hundredths(),
        }
    }
}

/// A complete invoice. Build it once; render both the PDF and the CII XML from
/// it so the two representations are guaranteed consistent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    /// Invoice number (BT-1).
    pub number: String,
    /// Issue date (BT-2) in `YYYY-MM-DD`.
    pub issue_date: String,
    /// Optional due date (BT-9) in `YYYY-MM-DD`, shown on the PDF.
    #[serde(default)]
    pub due_date: Option<String>,
    /// Document type code (BT-3). Defaults to `380` (commercial invoice).
    #[serde(default = "default_type_code")]
    pub type_code: String,
    /// ISO 4217 currency code (BT-5), e.g. `"EUR"`.
    pub currency: String,
    /// Optional currency symbol for the visual PDF (defaults from the code).
    #[serde(default)]
    pub currency_symbol: Option<String>,
    pub seller: Party,
    pub buyer: Party,
    pub lines: Vec<Line>,
    /// Optional free-text note (BT-22), shown on the PDF.
    #[serde(default)]
    pub note: Option<String>,
    /// Amount already paid (BT-113); subtracted from the grand total to give the
    /// amount due. Defaults to zero.
    #[serde(default)]
    pub prepaid: Amount,
    /// Document-level allowances/discounts (BG-20).
    #[serde(default)]
    pub allowances: Vec<AllowanceCharge>,
    /// Document-level charges, e.g. shipping or fees (BG-21).
    #[serde(default)]
    pub charges: Vec<AllowanceCharge>,
    /// Free-text payment terms (BT-20), e.g. `"30 days net"`.
    #[serde(default)]
    pub payment_terms: Option<String>,
}

/// One row of the VAT breakdown: all lines sharing a (category, rate) pair.
struct TaxGroup {
    category: String,
    rate: f64,
    basis: i64,
    tax: i64,
}

/// Derived monetary figures, all in hundredths, computed from the line items
/// and the document-level allowances/charges (the BT-106…BT-115 totals chain).
struct Computed {
    line_nets: Vec<i64>,
    /// Resolved amounts, aligned with `Invoice::allowances` / `::charges`.
    allowance_amounts: Vec<i64>,
    charge_amounts: Vec<i64>,
    groups: Vec<TaxGroup>,
    line_total: i64,      // BT-106
    allowance_total: i64, // BT-107
    charge_total: i64,    // BT-108
    tax_basis_total: i64, // BT-109
    tax_total: i64,       // BT-110
    grand_total: i64,     // BT-112
    due_payable: i64,     // BT-115
}

/// The breakdown row for a (category, rate) pair, created on first use.
fn group_entry<'a>(groups: &'a mut Vec<TaxGroup>, category: &str, rate: f64) -> &'a mut TaxGroup {
    let rate_key = (rate * 100.0).round() as i64;
    match groups
        .iter()
        .position(|g| g.category == category && (g.rate * 100.0).round() as i64 == rate_key)
    {
        Some(i) => &mut groups[i],
        None => {
            groups.push(TaxGroup {
                category: category.to_string(),
                rate,
                basis: 0,
                tax: 0,
            });
            groups.last_mut().unwrap()
        }
    }
}

impl Invoice {
    /// Compute line nets, the VAT breakdown, and all header totals. Pure
    /// arithmetic over the lines — this is the single place the numbers come
    /// from, shared by the PDF and the XML.
    fn compute(&self) -> Computed {
        let line_nets: Vec<i64> = self.lines.iter().map(Line::net_hundredths).collect();
        let line_total: i64 = line_nets.iter().sum();

        // Group by (category, rate), preserving first-seen order.
        let mut groups: Vec<TaxGroup> = Vec::new();
        for (line, &net) in self.lines.iter().zip(&line_nets) {
            group_entry(&mut groups, &line.vat_category, line.vat_rate).basis += net;
        }

        // Document-level allowances (BG-20) and charges (BG-21): resolve each
        // once (in hundredths), then adjust its (category, rate) group's basis.
        // A negative group basis (large allowance) is legal and kept as-is.
        let allowance_amounts: Vec<i64> = self
            .allowances
            .iter()
            .map(|ac| ac.resolve_hundredths(line_total))
            .collect();
        let charge_amounts: Vec<i64> = self
            .charges
            .iter()
            .map(|ac| ac.resolve_hundredths(line_total))
            .collect();
        for (ac, &amt) in self.allowances.iter().zip(&allowance_amounts) {
            group_entry(&mut groups, &ac.vat_category, ac.vat_rate).basis -= amt;
        }
        for (ac, &amt) in self.charges.iter().zip(&charge_amounts) {
            group_entry(&mut groups, &ac.vat_category, ac.vat_rate).basis += amt;
        }

        // Tax per group rounds once, after all basis adjustments (BR-CO-17).
        for g in &mut groups {
            g.tax = (g.basis as f64 * g.rate / 100.0).round() as i64;
        }

        let allowance_total: i64 = allowance_amounts.iter().sum();
        let charge_total: i64 = charge_amounts.iter().sum();
        let tax_basis_total = line_total - allowance_total + charge_total;
        let tax_total: i64 = groups.iter().map(|g| g.tax).sum();
        let grand_total = tax_basis_total + tax_total;
        let due_payable = grand_total - self.prepaid.hundredths();

        Computed {
            line_nets,
            allowance_amounts,
            charge_amounts,
            groups,
            line_total,
            allowance_total,
            charge_total,
            tax_basis_total,
            tax_total,
            grand_total,
            due_payable,
        }
    }

    /// Currency symbol for display: the explicit one, else a guess from the
    /// ISO code, else the code followed by a space.
    fn symbol(&self) -> String {
        if let Some(s) = &self.currency_symbol {
            return s.clone();
        }
        match self.currency.to_ascii_uppercase().as_str() {
            "EUR" => "€".to_string(),
            "USD" => "$".to_string(),
            "GBP" => "£".to_string(),
            "JPY" => "¥".to_string(),
            other => format!("{other} "),
        }
    }

    /// Map the invoice onto the JSON shape the bundled template interpolates:
    /// `invoice.*`, `company.*`, `customer.*`, `items[]`, `total.*`, `infos.*`.
    /// Returns the full `{ "data": { ... } }` document the render engine expects.
    pub fn to_render_data(&self) -> Value {
        let c = self.compute();
        let sym = self.symbol();
        let money = |h: i64| format!("{sym}{}", Amount::from_hundredths(h).format());

        let items: Vec<Value> = self
            .lines
            .iter()
            .zip(&c.line_nets)
            .map(|(line, &net)| {
                json!({
                    "name": line.name,
                    "qty": fmt_qty(line.quantity),
                    "amount": money(line.unit_price.hundredths()),
                    "total_amount": money(net),
                })
            })
            .collect();

        let discounts: Vec<Value> = self
            .allowances
            .iter()
            .zip(&c.allowance_amounts)
            .map(|(ac, &amt)| allowance_charge_json(ac, amt, &money))
            .collect();
        let charges: Vec<Value> = self
            .charges
            .iter()
            .zip(&c.charge_amounts)
            .map(|(ac, &amt)| allowance_charge_json(ac, amt, &money))
            .collect();

        json!({
            "data": {
                "invoice": {
                    "number": self.number,
                    "created_at": self.issue_date,
                    "due_date": self.due_date.clone().unwrap_or_default(),
                    "due_amount": money(c.due_payable),
                    "payment_terms": self.payment_terms.clone().unwrap_or_default(),
                    "type_code": self.type_code,
                    // Ready-made label — templates interpolate, they don't branch.
                    "type_label": if self.is_credit_note() { "Credit note" } else { "Invoice" },
                },
                "company": party_json(&self.seller, "name"),
                "customer": party_json(&self.buyer, "company"),
                "items": items,
                "discounts": discounts,
                "charges": charges,
                "total": {
                    "amount": money(c.line_total),
                    "discount": money(c.allowance_total),
                    "charges": money(c.charge_total),
                    "taxable": money(c.tax_basis_total),
                    "vat": money(c.tax_total),
                    "due_amount": money(c.due_payable),
                },
                "infos": { "text": self.note.clone().unwrap_or_default() },
                // Raw values (no currency symbol) for the EPC "scan-to-pay" QR.
                "payment": {
                    "name": self.seller.name,
                    "iban": self.seller.iban.clone().unwrap_or_default(),
                    "bic": self.seller.bic.clone().unwrap_or_default(),
                    "amount": Amount::from_hundredths(c.due_payable).format(),
                    "currency": self.currency,
                    "remittance": self.number,
                },
            }
        })
    }

    /// Render the invoice as EN 16931 Cross-Industry Invoice (CII) XML, with the
    /// VAT breakdown and totals computed from the lines. The `profile` only sets
    /// the `GuidelineSpecifiedDocumentContextParameter/ID` (BT-24); pass the same
    /// profile to the embed step.
    pub fn to_cii_xml(&self, profile: Profile) -> Result<String> {
        self.validate()?;
        let c = self.compute();
        let issue = cii_date("issue_date", &self.issue_date)?;
        let cur = &self.currency;

        let mut out = String::with_capacity(4096);
        out.push_str(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice
    xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"
    xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100"
    xmlns:qdt="urn:un:unece:uncefact:data:standard:QualifiedDataType:100"
    xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
"#,
        );

        // Document context + header.
        out.push_str("  <rsm:ExchangedDocumentContext>\n");
        out.push_str("    <ram:GuidelineSpecifiedDocumentContextParameter>\n");
        out.push_str(&format!(
            "      <ram:ID>{}</ram:ID>\n",
            esc(profile.guideline_id())
        ));
        out.push_str("    </ram:GuidelineSpecifiedDocumentContextParameter>\n");
        out.push_str("  </rsm:ExchangedDocumentContext>\n");

        out.push_str("  <rsm:ExchangedDocument>\n");
        out.push_str(&format!("    <ram:ID>{}</ram:ID>\n", esc(&self.number)));
        out.push_str(&format!(
            "    <ram:TypeCode>{}</ram:TypeCode>\n",
            esc(&self.type_code)
        ));
        out.push_str("    <ram:IssueDateTime>\n");
        out.push_str(&format!(
            "      <udt:DateTimeString format=\"102\">{issue}</udt:DateTimeString>\n"
        ));
        out.push_str("    </ram:IssueDateTime>\n");
        out.push_str("  </rsm:ExchangedDocument>\n");

        out.push_str("  <rsm:SupplyChainTradeTransaction>\n");

        // One IncludedSupplyChainTradeLineItem per line (BG-25).
        for (i, (line, &net)) in self.lines.iter().zip(&c.line_nets).enumerate() {
            let id = i + 1;
            out.push_str("    <ram:IncludedSupplyChainTradeLineItem>\n");
            out.push_str("      <ram:AssociatedDocumentLineDocument>\n");
            out.push_str(&format!("        <ram:LineID>{id}</ram:LineID>\n"));
            out.push_str("      </ram:AssociatedDocumentLineDocument>\n");
            out.push_str("      <ram:SpecifiedTradeProduct>\n");
            out.push_str(&format!(
                "        <ram:Name>{}</ram:Name>\n",
                esc(&line.name)
            ));
            out.push_str("      </ram:SpecifiedTradeProduct>\n");
            out.push_str("      <ram:SpecifiedLineTradeAgreement>\n");
            out.push_str("        <ram:NetPriceProductTradePrice>\n");
            out.push_str(&format!(
                "          <ram:ChargeAmount>{}</ram:ChargeAmount>\n",
                line.unit_price.format()
            ));
            out.push_str("        </ram:NetPriceProductTradePrice>\n");
            out.push_str("      </ram:SpecifiedLineTradeAgreement>\n");
            out.push_str("      <ram:SpecifiedLineTradeDelivery>\n");
            out.push_str(&format!(
                "        <ram:BilledQuantity unitCode=\"{}\">{}</ram:BilledQuantity>\n",
                esc(&line.unit_code),
                fmt_qty(line.quantity)
            ));
            out.push_str("      </ram:SpecifiedLineTradeDelivery>\n");
            out.push_str("      <ram:SpecifiedLineTradeSettlement>\n");
            out.push_str("        <ram:ApplicableTradeTax>\n");
            out.push_str("          <ram:TypeCode>VAT</ram:TypeCode>\n");
            out.push_str(&format!(
                "          <ram:CategoryCode>{}</ram:CategoryCode>\n",
                esc(&line.vat_category)
            ));
            out.push_str(&format!(
                "          <ram:RateApplicablePercent>{:.2}</ram:RateApplicablePercent>\n",
                line.vat_rate
            ));
            out.push_str("        </ram:ApplicableTradeTax>\n");
            out.push_str("        <ram:SpecifiedTradeSettlementLineMonetarySummation>\n");
            out.push_str(&format!(
                "          <ram:LineTotalAmount>{}</ram:LineTotalAmount>\n",
                Amount::from_hundredths(net).format()
            ));
            out.push_str("        </ram:SpecifiedTradeSettlementLineMonetarySummation>\n");
            out.push_str("      </ram:SpecifiedLineTradeSettlement>\n");
            out.push_str("    </ram:IncludedSupplyChainTradeLineItem>\n");
        }

        // Seller (BG-4) + Buyer (BG-7).
        out.push_str("    <ram:ApplicableHeaderTradeAgreement>\n");
        push_party(&mut out, "SellerTradeParty", &self.seller);
        push_party(&mut out, "BuyerTradeParty", &self.buyer);
        out.push_str("    </ram:ApplicableHeaderTradeAgreement>\n");

        out.push_str("    <ram:ApplicableHeaderTradeDelivery/>\n");

        // Totals (BG-22) + VAT breakdown (BG-23).
        out.push_str("    <ram:ApplicableHeaderTradeSettlement>\n");
        out.push_str(&format!(
            "      <ram:InvoiceCurrencyCode>{}</ram:InvoiceCurrencyCode>\n",
            esc(cur)
        ));
        for g in &c.groups {
            out.push_str("      <ram:ApplicableTradeTax>\n");
            out.push_str(&format!(
                "        <ram:CalculatedAmount>{}</ram:CalculatedAmount>\n",
                Amount::from_hundredths(g.tax).format()
            ));
            out.push_str("        <ram:TypeCode>VAT</ram:TypeCode>\n");
            out.push_str(&format!(
                "        <ram:BasisAmount>{}</ram:BasisAmount>\n",
                Amount::from_hundredths(g.basis).format()
            ));
            out.push_str(&format!(
                "        <ram:CategoryCode>{}</ram:CategoryCode>\n",
                esc(&g.category)
            ));
            out.push_str(&format!(
                "        <ram:RateApplicablePercent>{:.2}</ram:RateApplicablePercent>\n",
                g.rate
            ));
            out.push_str("      </ram:ApplicableTradeTax>\n");
        }

        // Document-level allowances (BG-20) then charges (BG-21) — the CII
        // schema requires them after the tax breakdown and before the payment
        // terms / monetary summation.
        for (ac, &amt) in self.allowances.iter().zip(&c.allowance_amounts) {
            push_allowance_charge(&mut out, ac, amt, c.line_total, false);
        }
        for (ac, &amt) in self.charges.iter().zip(&c.charge_amounts) {
            push_allowance_charge(&mut out, ac, amt, c.line_total, true);
        }

        // Payment terms (BT-20) / due date (BT-9). BR-CO-25: a positive amount
        // due requires one of the two to be present.
        if self.payment_terms.is_some() || self.due_date.is_some() {
            out.push_str("      <ram:SpecifiedTradePaymentTerms>\n");
            if let Some(terms) = &self.payment_terms {
                out.push_str(&format!(
                    "        <ram:Description>{}</ram:Description>\n",
                    esc(terms)
                ));
            }
            if let Some(due) = &self.due_date {
                let due = cii_date("due_date", due)?;
                out.push_str("        <ram:DueDateDateTime>\n");
                out.push_str(&format!(
                    "          <udt:DateTimeString format=\"102\">{due}</udt:DateTimeString>\n"
                ));
                out.push_str("        </ram:DueDateDateTime>\n");
            }
            out.push_str("      </ram:SpecifiedTradePaymentTerms>\n");
        }

        out.push_str("      <ram:SpecifiedTradeSettlementHeaderMonetarySummation>\n");
        out.push_str(&format!(
            "        <ram:LineTotalAmount>{}</ram:LineTotalAmount>\n",
            Amount::from_hundredths(c.line_total).format()
        ));
        // CII order trap: ChargeTotalAmount comes BEFORE AllowanceTotalAmount
        // (the opposite of UBL). Both are emitted only when used.
        if !self.charges.is_empty() {
            out.push_str(&format!(
                "        <ram:ChargeTotalAmount>{}</ram:ChargeTotalAmount>\n",
                Amount::from_hundredths(c.charge_total).format()
            ));
        }
        if !self.allowances.is_empty() {
            out.push_str(&format!(
                "        <ram:AllowanceTotalAmount>{}</ram:AllowanceTotalAmount>\n",
                Amount::from_hundredths(c.allowance_total).format()
            ));
        }
        out.push_str(&format!(
            "        <ram:TaxBasisTotalAmount>{}</ram:TaxBasisTotalAmount>\n",
            Amount::from_hundredths(c.tax_basis_total).format()
        ));
        out.push_str(&format!(
            "        <ram:TaxTotalAmount currencyID=\"{}\">{}</ram:TaxTotalAmount>\n",
            esc(cur),
            Amount::from_hundredths(c.tax_total).format()
        ));
        out.push_str(&format!(
            "        <ram:GrandTotalAmount>{}</ram:GrandTotalAmount>\n",
            Amount::from_hundredths(c.grand_total).format()
        ));
        // BR-CO-16: BT-115 = BT-112 − BT-113, so a non-zero prepaid amount
        // must be spelled out.
        if self.prepaid.hundredths() != 0 {
            out.push_str(&format!(
                "        <ram:TotalPrepaidAmount>{}</ram:TotalPrepaidAmount>\n",
                self.prepaid.format()
            ));
        }
        out.push_str(&format!(
            "        <ram:DuePayableAmount>{}</ram:DuePayableAmount>\n",
            Amount::from_hundredths(c.due_payable).format()
        ));
        out.push_str("      </ram:SpecifiedTradeSettlementHeaderMonetarySummation>\n");
        out.push_str("    </ram:ApplicableHeaderTradeSettlement>\n");

        out.push_str("  </rsm:SupplyChainTradeTransaction>\n");
        out.push_str("</rsm:CrossIndustryInvoice>\n");
        Ok(out)
    }

    /// Parse an invoice from JSON bytes.
    pub fn parse(bytes: &[u8]) -> Result<Invoice> {
        let invoice: Invoice = serde_json::from_slice(bytes).map_err(PdfError::from)?;
        invoice.validate()?;
        Ok(invoice)
    }

    /// Check invariants serde can't express: the BT-3 type-code allowlist and
    /// the amount-XOR-percent rule on allowances/charges. Called by [`parse`]
    /// and by [`to_cii_xml`] (for directly constructed invoices).
    ///
    /// [`parse`]: Invoice::parse
    /// [`to_cii_xml`]: Invoice::to_cii_xml
    pub fn validate(&self) -> Result<()> {
        if !TYPE_CODES.contains(&self.type_code.as_str()) {
            return Err(PdfError::Invoice(format!(
                "type_code {:?} is not a supported BT-3 code (accepted: {})",
                self.type_code,
                TYPE_CODES.join(", ")
            )));
        }
        for (kind, list) in [("allowances", &self.allowances), ("charges", &self.charges)] {
            for ac in list {
                match (ac.amount, ac.percent) {
                    (Some(_), Some(_)) | (None, None) => {
                        return Err(PdfError::Invoice(format!(
                            "{kind} entry {:?} must set exactly one of \"amount\" or \"percent\"",
                            ac.reason
                        )))
                    }
                    (None, Some(p)) if !p.is_finite() || p < 0.0 => {
                        return Err(PdfError::Invoice(format!(
                            "{kind} entry {:?} has an invalid percent: {p}",
                            ac.reason
                        )))
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Whether the document is a credit note (BT-3 = 381 or 261). CII has no
    /// separate credit-note document type: amounts stay positive and the type
    /// code carries the sign semantics.
    pub fn is_credit_note(&self) -> bool {
        matches!(self.type_code.as_str(), "381" | "261")
    }
}

/// Document type codes accepted for BT-3 (the Factur-X 1.0 subset): 380
/// commercial invoice, 381 credit note, 384 corrected invoice, 389
/// self-billed invoice, 261 self-billed credit note, 386 prepayment invoice.
const TYPE_CODES: [&str; 6] = ["380", "381", "384", "389", "261", "386"];

/// Emit one `SpecifiedTradeAllowanceCharge` (BG-20 allowance / BG-21 charge)
/// in CII child order: ChargeIndicator, CalculationPercent?, BasisAmount?,
/// ActualAmount, Reason, CategoryTradeTax.
fn push_allowance_charge(
    out: &mut String,
    ac: &AllowanceCharge,
    resolved: i64,
    basis: i64,
    is_charge: bool,
) {
    out.push_str("      <ram:SpecifiedTradeAllowanceCharge>\n");
    out.push_str(&format!(
        "        <ram:ChargeIndicator><udt:Indicator>{is_charge}</udt:Indicator></ram:ChargeIndicator>\n"
    ));
    if let Some(p) = ac.percent {
        out.push_str(&format!(
            "        <ram:CalculationPercent>{p:.2}</ram:CalculationPercent>\n"
        ));
        out.push_str(&format!(
            "        <ram:BasisAmount>{}</ram:BasisAmount>\n",
            Amount::from_hundredths(basis).format()
        ));
    }
    out.push_str(&format!(
        "        <ram:ActualAmount>{}</ram:ActualAmount>\n",
        Amount::from_hundredths(resolved).format()
    ));
    out.push_str(&format!(
        "        <ram:Reason>{}</ram:Reason>\n",
        esc(&ac.reason)
    ));
    out.push_str("        <ram:CategoryTradeTax>\n");
    out.push_str("          <ram:TypeCode>VAT</ram:TypeCode>\n");
    out.push_str(&format!(
        "          <ram:CategoryCode>{}</ram:CategoryCode>\n",
        esc(&ac.vat_category)
    ));
    out.push_str(&format!(
        "          <ram:RateApplicablePercent>{:.2}</ram:RateApplicablePercent>\n",
        ac.vat_rate
    ));
    out.push_str("        </ram:CategoryTradeTax>\n");
    out.push_str("      </ram:SpecifiedTradeAllowanceCharge>\n");
}

/// Emit a `<ram:{tag}>` trade party block (Name, optional phone contact, postal
/// address, optional VAT registration) in CII element order.
fn push_party(out: &mut String, tag: &str, p: &Party) {
    out.push_str(&format!("      <ram:{tag}>\n"));
    out.push_str(&format!("        <ram:Name>{}</ram:Name>\n", esc(&p.name)));
    if let Some(phone) = &p.phone {
        out.push_str("        <ram:DefinedTradeContact>\n");
        out.push_str("          <ram:TelephoneUniversalCommunication>\n");
        out.push_str(&format!(
            "            <ram:CompleteNumber>{}</ram:CompleteNumber>\n",
            esc(phone)
        ));
        out.push_str("          </ram:TelephoneUniversalCommunication>\n");
        out.push_str("        </ram:DefinedTradeContact>\n");
    }
    out.push_str("        <ram:PostalTradeAddress>\n");
    out.push_str(&format!(
        "          <ram:PostcodeCode>{}</ram:PostcodeCode>\n",
        esc(&p.postcode)
    ));
    out.push_str(&format!(
        "          <ram:LineOne>{}</ram:LineOne>\n",
        esc(&p.address_line)
    ));
    out.push_str(&format!(
        "          <ram:CityName>{}</ram:CityName>\n",
        esc(&p.city)
    ));
    out.push_str(&format!(
        "          <ram:CountryID>{}</ram:CountryID>\n",
        esc(&p.country)
    ));
    out.push_str("        </ram:PostalTradeAddress>\n");
    if let Some(vat) = &p.vat_id {
        out.push_str("        <ram:SpecifiedTaxRegistration>\n");
        out.push_str(&format!(
            "          <ram:ID schemeID=\"VA\">{}</ram:ID>\n",
            esc(vat)
        ));
        out.push_str("        </ram:SpecifiedTaxRegistration>\n");
    }
    out.push_str(&format!("      </ram:{tag}>\n"));
}

/// One allowance/charge row for the template (`discounts[]` / `charges[]`):
/// reason, formatted amount, and the percent when percent-based (else `""`).
fn allowance_charge_json(ac: &AllowanceCharge, resolved: i64, money: &dyn Fn(i64) -> String) -> Value {
    json!({
        "reason": ac.reason,
        "amount": money(resolved),
        "percent": ac.percent.map(fmt_qty).unwrap_or_default(),
    })
}

/// Build the per-party JSON the template expects. `name_key` is the field the
/// template reads the party name from (`name` for seller, `company` for buyer).
fn party_json(p: &Party, name_key: &str) -> Value {
    json!({
        name_key: p.name,
        "name": p.name,
        "address": p.address_line,
        "zipcode": p.postcode,
        "city": p.city,
        "country": p.country_display(),
        "phone": p.phone.clone().unwrap_or_default(),
    })
}

/// Format a quantity: drop the decimals when it is a whole number.
fn fmt_qty(q: f64) -> String {
    if q.fract() == 0.0 {
        format!("{}", q as i64)
    } else {
        format!("{q}")
    }
}

/// Convert a `YYYY-MM-DD` date into the CII `format="102"` form (`YYYYMMDD`),
/// validating the shape. `field` names the offending field in the error.
fn cii_date(field: &str, s: &str) -> Result<String> {
    let parts: Vec<&str> = s.trim().split('-').collect();
    let bad = || PdfError::Invoice(format!("{field} must be YYYY-MM-DD, got {s:?}"));
    if parts.len() != 3 {
        return Err(bad());
    }
    let (y, m, d) = (parts[0], parts[1], parts[2]);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(bad());
    }
    let valid = y
        .bytes()
        .chain(m.bytes())
        .chain(d.bytes())
        .all(|b| b.is_ascii_digit());
    if !valid {
        return Err(bad());
    }
    Ok(format!("{y}{m}{d}"))
}

/// XML-escape text for element content / attribute values.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Invoice {
        Invoice {
            number: "#12345".into(),
            issue_date: "2025-11-28".into(),
            due_date: Some("2025-12-28".into()),
            type_code: "380".into(),
            currency: "EUR".into(),
            currency_symbol: None,
            seller: Party {
                name: "PDFx".into(),
                address_line: "1 Rue de champs Elysées".into(),
                postcode: "75000".into(),
                city: "PARIS".into(),
                country: "FR".into(),
                country_name: Some("France".into()),
                phone: Some("+033 612348032".into()),
                vat_id: Some("FRXX999999999".into()),
                iban: Some("FR7630006000011234567890189".into()),
                bic: None,
            },
            buyer: Party {
                name: "John Doe".into(),
                address_line: "123 Main St".into(),
                postcode: "12345".into(),
                city: "NYC".into(),
                country: "US".into(),
                country_name: Some("USA".into()),
                phone: Some("+001 1234567890".into()),
                vat_id: None,
                iban: None,
                bic: None,
            },
            lines: vec![
                Line {
                    name: "Item 1".into(),
                    quantity: 1.0,
                    unit_code: "C62".into(),
                    unit_price: Amount::from_f64(100.0),
                    vat_rate: 20.0,
                    vat_category: "S".into(),
                },
                Line {
                    name: "Item 2".into(),
                    quantity: 2.0,
                    unit_code: "C62".into(),
                    unit_price: Amount::from_f64(200.0),
                    vat_rate: 20.0,
                    vat_category: "S".into(),
                },
            ],
            note: Some("lorem ipsum".into()),
            prepaid: Amount::default(),
            allowances: Vec::new(),
            charges: Vec::new(),
            payment_terms: None,
        }
    }

    fn allowance(reason: &str, amount: Option<f64>, percent: Option<f64>) -> AllowanceCharge {
        AllowanceCharge {
            reason: reason.into(),
            amount: amount.map(Amount::from_f64),
            percent,
            vat_rate: 20.0,
            vat_category: "S".into(),
        }
    }

    #[test]
    fn amount_formats_two_decimals() {
        assert_eq!(Amount::from_f64(100.0).format(), "100.00");
        assert_eq!(Amount::from_f64(12.5).format(), "12.50");
        assert_eq!(Amount::from_hundredths(5).format(), "0.05");
        assert_eq!(Amount::from_f64(-12.34).format(), "-12.34");
    }

    #[test]
    fn amount_deserializes_number_or_string() {
        assert_eq!(
            serde_json::from_str::<Amount>("100").unwrap(),
            Amount::from_hundredths(10000)
        );
        assert_eq!(
            serde_json::from_str::<Amount>("100.5").unwrap(),
            Amount::from_hundredths(10050)
        );
        assert_eq!(
            serde_json::from_str::<Amount>("\"100.50\"").unwrap(),
            Amount::from_hundredths(10050)
        );
    }

    #[test]
    fn totals_are_computed_and_consistent() {
        let c = sample().compute();
        assert_eq!(c.line_nets, vec![10000, 40000]); // 100 + 200*2
        assert_eq!(c.line_total, 50000); // 500.00
        assert_eq!(c.tax_total, 10000); // 20% of 500 = 100.00
        assert_eq!(c.grand_total, 60000); // 600.00
        assert_eq!(c.due_payable, 60000);
        assert_eq!(c.groups.len(), 1);
        assert_eq!(c.groups[0].basis, 50000);
    }

    #[test]
    fn multiple_vat_rates_group_separately() {
        let mut inv = sample();
        inv.lines[1].vat_rate = 10.0;
        let c = inv.compute();
        assert_eq!(c.groups.len(), 2);
        // 20% of 100 = 20.00 ; 10% of 400 = 40.00
        assert_eq!(c.tax_total, 6000);
        assert_eq!(c.grand_total, 56000);
    }

    #[test]
    fn cii_xml_has_computed_totals_and_structure() {
        let xml = sample().to_cii_xml(Profile::En16931).unwrap();
        assert!(xml.contains("<ram:ID>urn:cen.eu:en16931:2017</ram:ID>"));
        assert!(xml.contains("<ram:ID>#12345</ram:ID>"));
        assert!(xml.contains("<udt:DateTimeString format=\"102\">20251128</udt:DateTimeString>"));
        assert!(xml.contains("<ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode>"));
        assert!(xml.contains("<ram:GrandTotalAmount>600.00</ram:GrandTotalAmount>"));
        assert!(xml.contains("<ram:DuePayableAmount>600.00</ram:DuePayableAmount>"));
        assert!(xml.contains("<ram:TaxTotalAmount currencyID=\"EUR\">100.00</ram:TaxTotalAmount>"));
        assert!(xml.contains("<ram:ID schemeID=\"VA\">FRXX999999999</ram:ID>"));
        assert!(xml.contains("<ram:CountryID>FR</ram:CountryID>"));
        // Buyer has no VAT id → no second tax registration.
        assert_eq!(xml.matches("SpecifiedTaxRegistration").count(), 2);
    }

    #[test]
    fn render_data_matches_template_paths() {
        let v = sample().to_render_data();
        let d = &v["data"];
        assert_eq!(d["invoice"]["number"], "#12345");
        assert_eq!(d["invoice"]["due_amount"], "€600.00");
        assert_eq!(d["company"]["name"], "PDFx");
        assert_eq!(d["company"]["country"], "France"); // display name
        assert_eq!(d["customer"]["company"], "John Doe");
        assert_eq!(d["items"][1]["total_amount"], "€400.00");
        assert_eq!(d["total"]["vat"], "€100.00");
        assert_eq!(d["infos"]["text"], "lorem ipsum");
        // Payment block carries raw values (no currency symbol) for the QR.
        assert_eq!(d["payment"]["amount"], "600.00");
        assert_eq!(d["payment"]["currency"], "EUR");
        assert_eq!(d["payment"]["iban"], "FR7630006000011234567890189");
        assert_eq!(d["payment"]["remittance"], "#12345");
    }

    #[test]
    fn prepaid_reduces_due_amount() {
        let mut inv = sample();
        inv.prepaid = Amount::from_f64(100.0);
        let c = inv.compute();
        assert_eq!(c.grand_total, 60000);
        assert_eq!(c.due_payable, 50000); // 600 - 100
    }

    #[test]
    fn bad_date_is_rejected() {
        let mut inv = sample();
        inv.issue_date = "28/11/2025".into();
        assert!(inv.to_cii_xml(Profile::En16931).is_err());
    }

    #[test]
    fn allowance_amount_adjusts_basis_and_totals() {
        let mut inv = sample();
        inv.allowances = vec![allowance("Loyalty discount", Some(50.0), None)];
        let c = inv.compute();
        assert_eq!(c.line_total, 50000); // BT-106 unchanged
        assert_eq!(c.allowance_total, 5000); // BT-107 = 50.00
        assert_eq!(c.tax_basis_total, 45000); // BT-109 = 450.00
        assert_eq!(c.groups.len(), 1);
        assert_eq!(c.groups[0].basis, 45000);
        assert_eq!(c.tax_total, 9000); // 20% of 450
        assert_eq!(c.grand_total, 54000); // BT-112 = 540.00
        assert_eq!(c.due_payable, 54000);
    }

    #[test]
    fn percent_allowance_resolves_from_line_total() {
        let mut inv = sample();
        inv.allowances = vec![allowance("Volume discount", None, Some(10.0))];
        let c = inv.compute();
        assert_eq!(c.allowance_total, 5000); // 10% of 500.00

        let xml = inv.to_cii_xml(Profile::En16931).unwrap();
        assert!(xml.contains("<ram:CalculationPercent>10.00</ram:CalculationPercent>"));
        assert!(xml.contains("<ram:BasisAmount>500.00</ram:BasisAmount>"));
        assert!(xml.contains("<ram:ActualAmount>50.00</ram:ActualAmount>"));
    }

    #[test]
    fn charge_creates_own_vat_group() {
        let mut inv = sample();
        inv.charges = vec![AllowanceCharge {
            reason: "Shipping".into(),
            amount: Some(Amount::from_f64(20.0)),
            percent: None,
            vat_rate: 10.0, // no line uses 10% — must create a group
            vat_category: "S".into(),
        }];
        let c = inv.compute();
        assert_eq!(c.groups.len(), 2);
        assert_eq!(c.charge_total, 2000);
        assert_eq!(c.tax_basis_total, 52000); // 500 + 20
        assert_eq!(c.tax_total, 10200); // 20% of 500 + 10% of 20
        assert_eq!(c.grand_total, 62200);
    }

    #[test]
    fn allowance_requires_exactly_one_of_amount_or_percent() {
        let mut inv = sample();
        inv.allowances = vec![allowance("both", Some(10.0), Some(5.0))];
        assert!(inv.validate().is_err());
        inv.allowances = vec![allowance("neither", None, None)];
        assert!(inv.validate().is_err());
        inv.allowances = vec![allowance("negative", None, Some(-3.0))];
        assert!(inv.validate().is_err());
        inv.allowances = vec![allowance("ok", Some(10.0), None)];
        assert!(inv.validate().is_ok());
    }

    #[test]
    fn cii_xml_allowance_charge_order() {
        let mut inv = sample();
        inv.allowances = vec![allowance("Discount", Some(50.0), None)];
        inv.charges = vec![allowance("Shipping", Some(20.0), None)];
        inv.payment_terms = Some("30 days net".into());
        inv.prepaid = Amount::from_f64(100.0);
        let xml = inv.to_cii_xml(Profile::En16931).unwrap();

        // Header-settlement sequence: tax breakdown < allowance/charge <
        // payment terms < monetary summation.
        let idx = |needle: &str| {
            xml.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        };
        // The line items also carry ApplicableTradeTax, so anchor on the
        // header block explicitly.
        let header = idx("<ram:ApplicableHeaderTradeSettlement>");
        let tax = xml[header..].find("<ram:ApplicableTradeTax>").unwrap() + header;
        assert!(tax < idx("<ram:SpecifiedTradeAllowanceCharge>"));
        assert!(idx("<ram:SpecifiedTradeAllowanceCharge>") < idx("<ram:SpecifiedTradePaymentTerms>"));
        assert!(
            idx("<ram:SpecifiedTradePaymentTerms>")
                < idx("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>")
        );

        // Allowance/charge child order.
        assert!(idx("<ram:ChargeIndicator>") < idx("<ram:ActualAmount>"));
        assert!(idx("<ram:ActualAmount>") < idx("<ram:Reason>"));
        assert!(idx("<ram:Reason>") < idx("<ram:CategoryTradeTax>"));

        // Summation order: LineTotal < ChargeTotal < AllowanceTotal <
        // TaxBasisTotal, and GrandTotal < TotalPrepaid < DuePayable.
        assert!(idx("<ram:LineTotalAmount>") < idx("<ram:ChargeTotalAmount>"));
        assert!(idx("<ram:ChargeTotalAmount>") < idx("<ram:AllowanceTotalAmount>"));
        assert!(idx("<ram:AllowanceTotalAmount>") < idx("<ram:TaxBasisTotalAmount>"));
        assert!(idx("<ram:GrandTotalAmount>") < idx("<ram:TotalPrepaidAmount>"));
        assert!(idx("<ram:TotalPrepaidAmount>") < idx("<ram:DuePayableAmount>"));
    }

    #[test]
    fn payment_terms_and_due_date_emit_block() {
        let mut inv = sample();
        inv.payment_terms = Some("30 days net".into());
        let xml = inv.to_cii_xml(Profile::En16931).unwrap();
        assert!(xml.contains("<ram:Description>30 days net</ram:Description>"));
        // due_date is set in sample() → BT-9 present, format 102.
        assert!(xml.contains("<udt:DateTimeString format=\"102\">20251228</udt:DateTimeString>"));
        let desc = xml.find("<ram:Description>").unwrap();
        let due = xml.find("<ram:DueDateDateTime>").unwrap();
        assert!(desc < due, "Description must precede DueDateDateTime");
    }

    #[test]
    fn prepaid_emits_total_prepaid_amount() {
        let mut inv = sample();
        inv.prepaid = Amount::from_f64(100.0);
        let xml = inv.to_cii_xml(Profile::En16931).unwrap();
        // BR-CO-16 regression: BT-113 must be spelled out when non-zero.
        assert!(xml.contains("<ram:TotalPrepaidAmount>100.00</ram:TotalPrepaidAmount>"));
        assert!(xml.contains("<ram:DuePayableAmount>500.00</ram:DuePayableAmount>"));
    }

    #[test]
    fn credit_note_accepted_and_labeled() {
        let mut inv = sample();
        inv.type_code = "381".into();
        assert!(inv.is_credit_note());
        let xml = inv.to_cii_xml(Profile::En16931).unwrap();
        assert!(xml.contains("<ram:TypeCode>381</ram:TypeCode>"));
        // Amounts stay positive — the type code carries the semantics.
        assert!(xml.contains("<ram:GrandTotalAmount>600.00</ram:GrandTotalAmount>"));
        let v = inv.to_render_data();
        assert_eq!(v["data"]["invoice"]["type_label"], "Credit note");
    }

    #[test]
    fn unknown_type_code_rejected() {
        let mut inv = sample();
        inv.type_code = "999".into();
        assert!(inv.validate().is_err());
        assert!(inv.to_cii_xml(Profile::En16931).is_err());
    }

    #[test]
    fn render_data_exposes_discounts_and_charges() {
        let mut inv = sample();
        inv.allowances = vec![allowance("Volume discount", None, Some(10.0))];
        inv.charges = vec![allowance("Shipping", Some(20.0), None)];
        inv.payment_terms = Some("30 days net".into());
        let v = inv.to_render_data();
        let d = &v["data"];
        assert_eq!(d["total"]["discount"], "€50.00");
        assert_eq!(d["total"]["charges"], "€20.00");
        assert_eq!(d["total"]["taxable"], "€470.00");
        assert_eq!(d["discounts"][0]["reason"], "Volume discount");
        assert_eq!(d["discounts"][0]["percent"], "10");
        assert_eq!(d["charges"][0]["amount"], "€20.00");
        assert_eq!(d["charges"][0]["percent"], "");
        assert_eq!(d["invoice"]["payment_terms"], "30 days net");
        assert_eq!(d["invoice"]["type_label"], "Invoice");
    }

    #[test]
    fn no_new_fields_keeps_xml_free_of_new_elements() {
        let mut inv = sample();
        inv.due_date = None; // sample carries one; drop it to get the bare shape
        let xml = inv.to_cii_xml(Profile::En16931).unwrap();
        for absent in [
            "SpecifiedTradeAllowanceCharge",
            "SpecifiedTradePaymentTerms",
            "ChargeTotalAmount",
            "AllowanceTotalAmount",
            "TotalPrepaidAmount",
        ] {
            assert!(!xml.contains(absent), "{absent} must not appear");
        }
    }

    #[test]
    fn parse_accepts_enriched_invoice_json() {
        let json = r#"{
            "number": "F-1", "issue_date": "2026-07-01", "currency": "EUR",
            "seller": { "name": "ACME", "country": "FR" },
            "buyer": { "name": "Bob", "country": "FR" },
            "lines": [ { "name": "Widget", "unit_price": 100, "vat_rate": 20.0 } ],
            "allowances": [ { "reason": "Discount", "percent": 10, "vat_rate": 20.0 } ],
            "charges": [ { "reason": "Shipping", "amount": "20.00", "vat_rate": 20.0 } ],
            "payment_terms": "30 days net"
        }"#;
        let inv = Invoice::parse(json.as_bytes()).unwrap();
        assert_eq!(inv.allowances.len(), 1);
        assert_eq!(inv.charges[0].amount, Some(Amount::from_f64(20.0)));
        let c = inv.compute();
        assert_eq!(c.tax_basis_total, 11000); // 100 - 10 + 20
    }

    #[test]
    fn parse_rejects_invalid_allowance() {
        let json = r#"{
            "number": "F-1", "issue_date": "2026-07-01", "currency": "EUR",
            "seller": { "name": "ACME", "country": "FR" },
            "buyer": { "name": "Bob", "country": "FR" },
            "lines": [ { "name": "Widget", "unit_price": 100 } ],
            "allowances": [ { "reason": "broken" } ]
        }"#;
        assert!(Invoice::parse(json.as_bytes()).is_err());
    }
}
