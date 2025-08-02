use std::path::PathBuf;
use crate::helpers::{
    ApiData, AssetBundle,
    GameFiles, GameResources, GlobalCatalog,
    HashValue, MediaResources, Resource,
    ServerConfig, ServerRegion, TableResources,
};
use crate::utils::{file, json};

use anyhow::Result;
use baad_core::{errors::{ErrorContext, ErrorExt}, info, success};
use bacy::{Packing, MediaCatalog, TableCatalog};
use reqwest::Client;
use std::rc::Rc;

struct Paths {
    ios_asset_path: PathBuf,
	android_asset_path: PathBuf,
    table_path: PathBuf,
    media_path: PathBuf,
    game_path: PathBuf,
    resource_path: PathBuf,
    api_path: PathBuf
}

pub struct CatalogParser {
    client: Client,
    config: Rc<ServerConfig>,
    paths: Paths
}

impl CatalogParser {
    pub fn new(config: Rc<ServerConfig>) -> Result<Self> {
        let data_dir = file::data_dir()?;
        let api_path = data_dir.join("api_data.json");

        let catalog_dir = match config.region {
            ServerRegion::Global => data_dir.join("catalog/global"),
            ServerRegion::Japan =>  data_dir.join("catalog/japan"),
        };
        
		let android_asset_path = catalog_dir.join("BundlePackingInfo-Android.json");
        let ios_asset_path = catalog_dir.join("BundlePackingInfo-iOS.json");
        let table_path = catalog_dir.join("TableCatalog.json");
        let media_path = catalog_dir.join("MediaCatalog.json");
        let game_path = catalog_dir.join("GameFiles.json");
        let resource_path = catalog_dir.join("Resources.json");
        
        Ok(Self {
            client: Client::new(),
            config,
            paths: Paths { ios_asset_path, android_asset_path, table_path, media_path, game_path, resource_path, api_path }
        })
    }

    async fn japan_data(&self, catalog_url: &str) -> Result<()> {
		let android_asset_data = self
            .client
            .get(format!("{}/Android_PatchPack/BundlePackingInfo.json", catalog_url))
            .send()
            .await
            .handle_errors()?
            .json::<Packing>()
            .await
            .handle_errors()?;
        
        json::save_json(
            &self.paths.android_asset_path,
            &android_asset_data
        )
        .await?;
		
		let ios_asset_data = self
            .client
            .get(format!("{}/iOS_PatchPack/BundlePackingInfo.json", catalog_url))
            .send()
            .await
            .handle_errors()?
            .json::<Packing>()
            .await
            .handle_errors()?;

		json::save_json(
            &self.paths.ios_asset_path,
            &ios_asset_data
        )
        .await?;

        success!("Saved AssetBundles catalog");

        let table_bytes = self
            .client
            .get(format!("{}/TableBundles/TableCatalog.bytes", catalog_url))
            .send()
            .await
            .handle_errors()?
            .bytes()
            .await
            .handle_errors()?;
        let table_data = TableCatalog::deserialize(&table_bytes, catalog_url).handle_errors()?;

        json::save_json(
            &self.paths.table_path,
            &table_data,
        )
            .await?;

        success!("Saved TableBundles catalog");

        let media_bytes = self
            .client
            .get(format!(
                "{}/MediaResources/Catalog/MediaCatalog.bytes",
                catalog_url
            ))
            .send()
            .await
            .handle_errors()?
            .bytes()
            .await
            .handle_errors()?;
        let media_data = MediaCatalog::deserialize(&media_bytes, catalog_url).handle_errors()?;

        json::save_json(
            &self.paths.media_path,
            &media_data,
        )
        .await?;

        success!("Saved MediaResources catalog");

        Ok(())
    }

    async fn japan_gamefiles(&self, catalog_url: &str) -> Result<()> {
		let ios_bundle_info: Packing =
		    json::load_json(&self.paths.ios_asset_path).await?;
        let android_bundle_info: Packing =
            json::load_json(&self.paths.android_asset_path).await?;
        let table_catalog: TableCatalog =
            json::load_json(&self.paths.table_path).await?;
        let media_catalog: MediaCatalog =
            json::load_json(&self.paths.media_path).await?;

        let game_resources = GameResources {
			android_asset_bundles: android_bundle_info.full_patch_packs.iter()
                .chain(android_bundle_info.update_packs.iter())
                .map(|patch| GameFiles {
                    url: format!("{}/Android_PatchPack/{}", catalog_url, patch.pack_name),
                    path: format!("AndroidAssetBundles/{}", patch.pack_name),
                    hash: HashValue::Crc(patch.crc),
                    size: patch.pack_size,
                })
                .collect(),
				
			ios_asset_bundles: ios_bundle_info.full_patch_packs.iter()
                .chain(ios_bundle_info.update_packs.iter())
                .map(|patch| GameFiles {
                    url: format!("{}/iOS_PatchPack/{}", catalog_url, patch.pack_name),
                    path: format!("iOSAssetBundles/{}", patch.pack_name),
                    hash: HashValue::Crc(patch.crc),
                    size: patch.pack_size,
                })
                .collect(),

            table_bundles: table_catalog
                .table
                .into_values()
                .map(|entry| GameFiles {
                    url: format!("{}/TableBundles/{}", catalog_url, entry.name),
                    path: format!("TableBundles/{}", entry.name),
                    hash: HashValue::Crc(entry.crc),
                    size: entry.size,
                })
                .collect(),

            media_resources: media_catalog
                .table
                .into_values()
                .map(|entry| {
                    let path = entry.path.replace('\\', "/");
                    GameFiles {
                        url: format!("{}/MediaResources/{}", catalog_url, path),
                        path: format!("MediaResources/{}", path),
                        hash: HashValue::Crc(entry.crc),
                        size: entry.bytes,
                    }
                })
                .collect(),
        };

        json::save_json(
            &self.paths.game_path,
            &game_resources
        )
        .await?;

        success!("Saved GameFiles");
        
        Ok(())
    }

    async fn global_data(&self, resources: &[Resource]) -> Result<()> {
        let android_asset_bundles: Vec<Resource> = resources
            .iter()
            .filter(|r| r.resource_path.contains("/Android/"))
            .cloned()
            .collect();
			
		let ios_asset_bundles: Vec<Resource> = resources
            .iter()
            .filter(|r| r.resource_path.contains("/iOS/"))
            .cloned()
            .collect();

        let media_resources: Vec<Resource> = resources
            .iter()
            .filter(|r| r.resource_path.contains("/MediaResources/"))
            .cloned()
            .collect();

        let table_bundles: Vec<Resource> = resources
            .iter()
            .filter(|r| r.resource_path.contains("/TableBundles/"))
            .cloned()
            .collect();
        if !android_asset_bundles.is_empty() {
            let android_asset_data = AssetBundle { asset_bundles: android_asset_bundles };
            json::save_json(
                &self.paths.android_asset_path,
                &android_asset_data
            )
            .await?;

            success!("Saved Android AssetBundles catalog");
        }
        
        if !ios_asset_bundles.is_empty() {
            let ios_asset_data = AssetBundle { asset_bundles: ios_asset_bundles };
            json::save_json(
                &self.paths.ios_asset_path,
                &ios_asset_data
            )
            .await?;

            success!("Saved iOS AssetBundles catalog");
        }

        if !table_bundles.is_empty() {
            let table_data = TableResources { table_bundles };
            json::save_json(
                &self.paths.table_path,
                &table_data
            )
            .await?;

            success!("Saved TableBundles catalog");
        }

        if !media_resources.is_empty() {
            let media_data = MediaResources { media_resources };
            json::save_json(
                &self.paths.media_path,
                &media_data
            )
                .await?;

            success!("Saved MediaResources catalog");
        }

        Ok(())
    }

    async fn global_gamefiles(&self, catalog_url: &str, resources: &[Resource]) -> Result<()> {
        let game_resources = GameResources {
            android_asset_bundles: resources
                .iter()
                .filter(|r| r.resource_path.contains("/Android/"))
                .map(|r| self.resource_to_gamefiles(r, catalog_url, "AndroidAssetBundles"))
                .collect(),

            ios_asset_bundles: resources
                .iter()
                .filter(|r| r.resource_path.contains("/iOS/"))
                .map(|r| self.resource_to_gamefiles(r, catalog_url, "iOSAssetBundles"))
                .collect(),

            table_bundles: resources
                .iter()
                .filter(|r| r.resource_path.contains("/TableBundles/"))
                .map(|r| self.resource_to_gamefiles(r, catalog_url, "TableBundles"))
                .collect(),

            media_resources: resources
                .iter()
                .filter(|r| r.resource_path.contains("/MediaResources/"))
                .map(|r| self.resource_to_gamefiles(r, catalog_url, "MediaResources"))
                .collect(),
        };

        json::save_json(
            &self.paths.game_path,
            &game_resources
        ).await?;

        success!("Saved GameFiles");
        
        Ok(())
    }

    fn resource_to_gamefiles(&self, resource: &Resource, catalog_url: &str, prefix: &str) -> GameFiles {
        GameFiles {
            url: format!("{}/{}", catalog_url, resource.resource_path),
            path: format!("{}/{}", prefix, resource.resource_path),
            hash: HashValue::Md5(resource.resource_hash.clone()),
            size: resource.resource_size,
        }
    }

    pub async fn process_catalogs(&self) -> Result<()> {
        let api_data: ApiData = json::load_json(&self.paths.api_path).await?;
        
        info!("Processing catalogs...");

        match self.config.region {
            ServerRegion::Japan => {
                let catalog_url = &api_data.japan.catalog_url;

                if catalog_url.is_empty() {
                    return None.error_context("Japan catalog URL is empty - run CatalogFetcher first");
                }

                self.japan_data(catalog_url).await?;
                self.japan_gamefiles(catalog_url).await?;
            }
            ServerRegion::Global => {
                let resources: GlobalCatalog =
                    json::load_json(&self.paths.resource_path).await?;
                let catalog_url = &api_data
                    .global
                    .catalog_url
                    .trim_end_matches("/resource-data.json");

                if catalog_url.is_empty() {
                    return None.error_context("Global catalog URL is empty - run CatalogFetcher first");
                }

                self.global_data(&resources.resources).await?;
                self.global_gamefiles(catalog_url, &resources.resources)
                    .await?;
            }
        }

        Ok(())
    }
}