//! Shared HTTP endpoint config (base / URL template / prefix / headers / agent).
//!
//! Used by both [`crate::HttpChunkSource`] (GET) and [`crate::HttpChunkSink`] (PUT)
//! so read and write expand to the same key layout.

use crate::template::{TemplateCtx, TemplateError, expand_template, normalize_prefix};
use chunkforge_store::ChunkId;
use std::time::Duration;
use ureq::Agent;

/// Internal shared HTTP endpoint: templates + agent for one base URL.
#[derive(Debug, Clone)]
pub(crate) struct HttpEndpoint {
    pub(crate) base: String,
    pub(crate) agent: Agent,
    pub(crate) url_template: String,
    pub(crate) header_templates: Vec<(String, String)>,
    pub(crate) prefix: String,
}

impl HttpEndpoint {
    /// Validate templates (all-zero id) and build agent.
    pub(crate) fn build(
        base: impl Into<String>,
        timeout: Option<Duration>,
        url_template: String,
        header_templates: Vec<(String, String)>,
        prefix: impl Into<String>,
    ) -> Result<Self, TemplateError> {
        let base = base.into().trim_end_matches('/').to_string();
        let prefix = normalize_prefix(&prefix.into());
        let fake_id = ChunkId::from_bytes([0u8; 32]);
        let ctx = TemplateCtx {
            base: &base,
            id: &fake_id,
            prefix: &prefix,
        };
        expand_template(&url_template, &ctx)?;
        for (_name, value_tmpl) in &header_templates {
            expand_template(value_tmpl, &ctx)?;
        }

        let mut config = Agent::config_builder();
        if let Some(t) = timeout {
            config = config.timeout_global(Some(t));
        }
        let agent: Agent = config.build().into();
        Ok(Self {
            base,
            agent,
            url_template,
            header_templates,
            prefix,
        })
    }

    pub(crate) fn template_ctx<'a>(&'a self, id: &'a ChunkId) -> TemplateCtx<'a> {
        TemplateCtx {
            base: &self.base,
            id,
            prefix: &self.prefix,
        }
    }

    pub(crate) fn expand_url(&self, id: &ChunkId) -> Result<String, TemplateError> {
        expand_template(&self.url_template, &self.template_ctx(id))
    }

    pub(crate) fn expand_headers(
        &self,
        id: &ChunkId,
    ) -> Result<Vec<(String, String)>, TemplateError> {
        let ctx = self.template_ctx(id);
        self.header_templates
            .iter()
            .map(|(name, tmpl)| {
                let value = expand_template(tmpl, &ctx)?;
                Ok((name.clone(), value))
            })
            .collect()
    }

    /// Absolute URL for `id` (infallible after successful build, barring lost env vars).
    pub(crate) fn url_for(&self, id: &ChunkId) -> String {
        self.expand_url(id)
            .expect("url_template validated at build; env vars must remain set")
    }
}
