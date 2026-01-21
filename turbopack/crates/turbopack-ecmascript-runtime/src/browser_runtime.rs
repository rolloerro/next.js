use std::{fmt::Display, io::Write};

use anyhow::Result;
use indoc::writedoc;
use turbo_rcstr::{RcStr, rcstr};
use turbo_tasks::{ResolvedVc, Vc};
use turbopack_core::{
    code_builder::{Code, CodeBuilder},
    context::AssetContext,
    environment::{ChunkLoading, Environment},
};
use turbopack_ecmascript::utils::StringifyJs;

use crate::{
    ChunkSuffix, RuntimeType, asset_context::get_runtime_asset_context, embed_js::embed_static_code,
};

#[turbo_tasks::value]
pub struct BrowserRuntimeGlobals {
    chunk_base_path: ResolvedVc<Option<RcStr>>,
    chunk_suffix: ResolvedVc<ChunkSuffix>,
    output_root_to_root_path: ResolvedVc<RcStr>,
    worker_forwarded_globals: ResolvedVc<Vec<RcStr>>,
}

#[turbo_tasks::value_impl]
impl BrowserRuntimeGlobals {
    #[turbo_tasks::function]
    pub async fn new(
        chunk_base_path: Vc<Option<RcStr>>,
        chunk_suffix: Vc<ChunkSuffix>,
        output_root_to_root_path: Vc<RcStr>,
        worker_forwarded_globals: Vc<Vec<RcStr>>,
    ) -> Result<Vc<Self>> {
        Ok(BrowserRuntimeGlobals {
            chunk_base_path: chunk_base_path.to_resolved().await?,
            chunk_suffix: chunk_suffix.to_resolved().await?,
            output_root_to_root_path: output_root_to_root_path.to_resolved().await?,
            worker_forwarded_globals: worker_forwarded_globals.to_resolved().await?,
        }
        .cell())
    }

    #[turbo_tasks::function]
    pub async fn code(&self, is_edge: bool) -> Result<Vc<Code>> {
        let chunk_base_path = self.chunk_base_path.await?;
        let chunk_base_path = chunk_base_path.as_ref().map_or("", |s| s.as_str());
        let output_root_to_root_path = self.output_root_to_root_path.await?;
        let worker_forwarded_globals = self.worker_forwarded_globals.await?;
        let chunk_suffix = self.chunk_suffix.await?;

        let chunk_suffix_str: &dyn Display = match &*chunk_suffix {
            ChunkSuffix::None => &r#""""#,
            ChunkSuffix::Constant(suffix) => &StringifyJs(suffix.as_str()),
            ChunkSuffix::FromScriptSrc if is_edge => {
                panic!("ChunkSuffix::FromScriptSrc is not supported in Edge runtimes");
            }
            ChunkSuffix::FromScriptSrc => &"getChunkSuffixFromScriptSrc()",
        };
        let mut code = CodeBuilder::default();
        writedoc!(
            code,
            r#"
            const CHUNK_BASE_PATH = {};
            const RELATIVE_ROOT_PATH = {};
            const RUNTIME_PUBLIC_PATH = {};
            const CHUNK_SUFFIX = {};
            const WORKER_FORWARDED_GLOBALS = {};
            "#,
            StringifyJs(chunk_base_path),
            StringifyJs(output_root_to_root_path.as_str()),
            StringifyJs(chunk_base_path),
            chunk_suffix_str,
            StringifyJs(&*worker_forwarded_globals),
        )?;
        Ok(Code::cell(code.build()))
    }
}

/// Returns the code for the ECMAScript runtime.
#[turbo_tasks::function]
pub async fn get_browser_runtime_code(
    environment: ResolvedVc<Environment>,
    runtime_globals: Vc<BrowserRuntimeGlobals>,
    runtime_type: RuntimeType,
    generate_source_map: bool,
) -> Result<Vc<Code>> {
    let asset_context = get_runtime_asset_context(*environment).resolve().await?;

    let shared_runtime_utils_code = embed_static_code(
        asset_context,
        rcstr!("shared/runtime-utils.ts"),
        generate_source_map,
    );

    let mut runtime_base_code = vec!["browser/runtime/base/runtime-base.ts"];
    match runtime_type {
        RuntimeType::Production => runtime_base_code.push("browser/runtime/base/build-base.ts"),
        RuntimeType::Development => {
            runtime_base_code.push("browser/runtime/base/dev-base.ts");
        }
        #[cfg(feature = "test")]
        RuntimeType::Dummy => {
            panic!("This configuration is not supported in the browser runtime")
        }
    }

    let chunk_loading = &*asset_context
        .compile_time_info()
        .environment()
        .chunk_loading()
        .await?;

    let mut runtime_backend_code = vec![];
    match (chunk_loading, runtime_type) {
        (ChunkLoading::Edge, RuntimeType::Development) => {
            runtime_backend_code.push("browser/runtime/edge/runtime-backend-edge.ts");
            runtime_backend_code.push("browser/runtime/edge/dev-backend-edge.ts");
        }
        (ChunkLoading::Edge, RuntimeType::Production) => {
            runtime_backend_code.push("browser/runtime/edge/runtime-backend-edge.ts");
        }
        // This case should never be hit.
        (ChunkLoading::NodeJs, _) => {
            panic!("Node.js runtime is not supported in the browser runtime!")
        }
        (ChunkLoading::Dom, RuntimeType::Development) => {
            runtime_backend_code.push("browser/runtime/dom/runtime-backend-dom.ts");
            runtime_backend_code.push("browser/runtime/dom/dev-backend-dom.ts");
        }
        (ChunkLoading::Dom, RuntimeType::Production) => {
            runtime_backend_code.push("browser/runtime/dom/runtime-backend-dom.ts");
        }

        #[cfg(feature = "test")]
        (_, RuntimeType::Dummy) => {
            panic!("This configuration is not supported in the browser runtime")
        }
    };

    let mut code: CodeBuilder = CodeBuilder::default();
    writedoc!(
        code,
        r#"
            (() => {{
            if (!Array.isArray(globalThis.TURBOPACK)) {{
                return;
            }}

        "#,
    )?;

    code.push_code(
        &*runtime_globals
            .code(chunk_loading == &ChunkLoading::Edge)
            .await?,
    );
    code.push_code(&*shared_runtime_utils_code.await?);
    for runtime_code in runtime_base_code {
        code.push_code(
            &*embed_static_code(asset_context, runtime_code.into(), generate_source_map).await?,
        );
    }

    if *environment.supports_commonjs_externals().await? {
        code.push_code(
            &*embed_static_code(
                asset_context,
                rcstr!("shared-node/base-externals-utils.ts"),
                generate_source_map,
            )
            .await?,
        );
    }
    if *environment.node_externals().await? {
        code.push_code(
            &*embed_static_code(
                asset_context,
                rcstr!("shared-node/node-externals-utils.ts"),
                generate_source_map,
            )
            .await?,
        );
    }
    if *environment.supports_wasm().await? {
        code.push_code(
            &*embed_static_code(
                asset_context,
                rcstr!("shared-node/node-wasm-utils.ts"),
                generate_source_map,
            )
            .await?,
        );
    }

    for backend_code in runtime_backend_code {
        code.push_code(
            &*embed_static_code(asset_context, backend_code.into(), generate_source_map).await?,
        );
    }

    // Registering chunks and chunk lists depends on the BACKEND variable, which is set by the
    // specific runtime code, hence it must be appended after it.
    writedoc!(
        code,
        r#"
            const chunksToRegister = globalThis.TURBOPACK;
            globalThis.TURBOPACK = {{ push: registerChunk }};
            chunksToRegister.forEach(registerChunk);
        "#
    )?;
    if matches!(runtime_type, RuntimeType::Development) {
        writedoc!(
            code,
            r#"
            const chunkListsToRegister = globalThis.TURBOPACK_CHUNK_LISTS || [];
            globalThis.TURBOPACK_CHUNK_LISTS = {{ push: registerChunkList }};
            chunkListsToRegister.forEach(registerChunkList);
        "#
        )?;
    }
    writedoc!(
        code,
        r#"
            }})();
        "#
    )?;

    Ok(Code::cell(code.build()))
}
