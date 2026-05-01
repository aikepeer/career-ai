//! `ServerHandler` trait impl — MCP lifecycle hooks (info, resource listing,
//! resource reading). Kept separate from the `#[tool_router]` impl block so
//! this file can focus on the resource/lifecycle surface.

use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;
use rmcp::{tool_handler, ServerHandler};

use crate::error::McpServerError;

use super::CareerAiServer;

#[tool_handler]
impl ServerHandler for CareerAiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "career-ai pipeline as MCP. Discover jobs, score them against the \
             profile, tailor + render resume/cover letter, and submit through \
             a dry-run gate. Tools: careerai_profile_status, careerai_discover, \
             careerai_shortlist, careerai_tailor, careerai_render, \
             careerai_apply (defaults to dry_run; real submit needs an \
             explicit confirm token), careerai_inspect, careerai_digest. \
             Resources: careerai://profile, careerai://shortlist/{date}, \
             careerai://artifacts/{application_id}."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: self.list_resources_static(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: self.list_resource_templates_static(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri.as_str();

        // careerai://profile
        if uri == "careerai://profile" {
            return self.read_profile_resource(uri).await.map_err(Into::into);
        }

        // careerai://shortlist/{date}
        if let Some(rest) = uri.strip_prefix("careerai://shortlist/") {
            let date = rest.split('?').next().unwrap_or(rest);
            return self
                .read_shortlist_resource(uri, date)
                .await
                .map_err(Into::into);
        }

        // careerai://artifacts/{application_id}
        if let Some(rest) = uri.strip_prefix("careerai://artifacts/") {
            let app_id = rest.split('?').next().unwrap_or(rest);
            if app_id.is_empty() {
                return Err(McpServerError::ResourceNotFound(uri.to_string()).into());
            }
            return self
                .read_artifacts_resource(uri, app_id)
                .await
                .map_err(Into::into);
        }

        Err(McpServerError::ResourceNotFound(uri.to_string()).into())
    }
}
