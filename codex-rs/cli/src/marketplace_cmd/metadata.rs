use super::MarketplaceSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MarketplaceInstallMetadata {
    source: InstalledMarketplaceSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InstalledMarketplaceSource {
    Git {
        url: String,
        ref_name: Option<String>,
        sparse_paths: Vec<String>,
    },
    Path {
        path: String,
    },
    ManifestUrl {
        url: String,
    },
}

impl MarketplaceInstallMetadata {
    pub(super) fn from_source(source: &MarketplaceSource, sparse_paths: &[String]) -> Self {
        let source = match source {
            MarketplaceSource::Git { url, ref_name } => InstalledMarketplaceSource::Git {
                url: url.clone(),
                ref_name: ref_name.clone(),
                sparse_paths: sparse_paths.to_vec(),
            },
            MarketplaceSource::Path { path } => InstalledMarketplaceSource::Path {
                path: path.display().to_string(),
            },
            MarketplaceSource::ManifestUrl { url } => {
                InstalledMarketplaceSource::ManifestUrl { url: url.clone() }
            }
        };
        Self { source }
    }

    pub(super) fn config_source_type(&self) -> &'static str {
        match &self.source {
            InstalledMarketplaceSource::Git { .. } => "git",
            InstalledMarketplaceSource::Path { .. } => "path",
            InstalledMarketplaceSource::ManifestUrl { .. } => "manifest_url",
        }
    }

    pub(super) fn config_source(&self) -> String {
        match &self.source {
            InstalledMarketplaceSource::Git { url, .. } => url.clone(),
            InstalledMarketplaceSource::Path { path } => path.clone(),
            InstalledMarketplaceSource::ManifestUrl { url } => url.clone(),
        }
    }

    pub(super) fn ref_name(&self) -> Option<&str> {
        match &self.source {
            InstalledMarketplaceSource::Git { ref_name, .. } => ref_name.as_deref(),
            InstalledMarketplaceSource::Path { .. }
            | InstalledMarketplaceSource::ManifestUrl { .. } => None,
        }
    }

    pub(super) fn sparse_paths(&self) -> &[String] {
        match &self.source {
            InstalledMarketplaceSource::Git { sparse_paths, .. } => sparse_paths,
            InstalledMarketplaceSource::Path { .. }
            | InstalledMarketplaceSource::ManifestUrl { .. } => &[],
        }
    }
}
