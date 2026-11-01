//! The domains as text: `domains.toml`, and Copy as Markdown.

use super::*;

impl Domains {
    /// As `domains.toml` says it: one table per domain with its members
    /// one to a line, then the flows, then Shared.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        for domain in &self.domains {
            out.push_str("[[domain]]\n");
            out.push_str(&format!("name = {}\n", quoted(&domain.name)));
            out.push_str(&format!("purpose = {}\n", quoted(&domain.purpose)));
            out.push_str(&format!("offers = {}\n", inline_list(&domain.offers)));
            out.push_str(&format!("why = {}\n", quoted(&domain.why)));
            out.push_str(&format!("members = {}\n\n", long_list(&domain.members)));
        }
        for flow in &self.flows {
            out.push_str("[[flow]]\n");
            out.push_str(&format!("from = {}\n", quoted(&flow.from)));
            out.push_str(&format!("to = {}\n", quoted(&flow.to)));
            out.push_str(&format!("label = {}\n\n", quoted(&flow.label)));
        }
        out.push_str("[shared]\n");
        out.push_str(&format!("members = {}\n", long_list(&self.shared.members)));
        out
    }

    /// Each domain's name, purpose and what it offers, then the flows, as
    /// Markdown to paste into any doc.
    pub fn markdown(&self, project: &str) -> String {
        let mut out = format!("# {project}: domains\n");
        for domain in &self.domains {
            out.push_str(&format!("\n## {}\n\n", domain.name));
            if !domain.purpose.is_empty() {
                out.push_str(&format!("{}\n", domain.purpose));
            }
            if !domain.offers.is_empty() {
                out.push_str("\nOffers:\n");
                for offer in &domain.offers {
                    out.push_str(&format!("- {offer}\n"));
                }
            }
        }
        if !self.flows.is_empty() {
            out.push_str("\n## How they work together\n\n");
            for flow in &self.flows {
                let label = if flow.label.is_empty() {
                    "works with"
                } else {
                    &flow.label
                };
                out.push_str(&format!("- {} {label} {}\n", flow.from, flow.to));
            }
        }
        out
    }
}
