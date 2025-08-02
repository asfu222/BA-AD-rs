use crate::apk::{ApkExtractor, ApkFetcher};
use crate::catalog::{CatalogFetcher, CatalogParser};
use crate::cli::args::{Args, Commands, DownloadArgs, RegionCommands};
use crate::download::{FilterMethod, ResourceCategory, ResourceDownloadBuilder, ResourceFilter};
use crate::helpers::{ServerConfig, ServerRegion};
use crate::utils::file;

use anyhow::Result;
use baad_core::{errors::ErrorContext, info, success};
use std::rc::Rc;

pub struct CommandHandler {
    args: Args,
}

impl CommandHandler {
    fn new(args: Args) -> Result<Self> {
        Ok(Self { args })
    }

    async fn handle(&self) -> Result<()> {
        if self.args.clean {
            info!("Cleaning data...");

            let data_dir = file::data_dir()?;
            file::clear_all(&data_dir).await?;

            success!("Data cleared");
        }

        match &self.args.command {
            Some(Commands::Download { region }) => self.handle_download(region).await,
            None => {
                if self.args.update {
                    self.handle_update().await?;
                } else if !self.args.clean {
                    info!("No command specified. Use --help for usage information.");
                }

                Ok(())
            }
        }
    }

    async fn handle_download(&self, region: &RegionCommands) -> Result<()> {
        match region {
            RegionCommands::Global(download_args) => {
                self.execute_download(ServerRegion::Global, download_args)
                    .await
            }
            RegionCommands::Japan(download_args) => {
                self.execute_download(ServerRegion::Japan, download_args)
                    .await
            }
        }
    }

    async fn handle_update(&self) -> Result<()> {
        info!("Forcing update...");

        let server_config = ServerConfig::new(ServerRegion::Japan)?;
        let apk_fetcher = ApkFetcher::new(server_config.clone())?;

        apk_fetcher.download_apk(true).await?;

        Ok(())
    }

    async fn execute_download(&self, region: ServerRegion, args: &DownloadArgs) -> Result<()> {
        let server_config = ServerConfig::new(region)?;
        let apk_fetcher = ApkFetcher::new(server_config.clone())?;

        let should_process_catalogs = match region {
            ServerRegion::Japan => {
                let should_process = self.handle_japan(&apk_fetcher).await?;

                if should_process {
                    apk_fetcher.download_apk(self.args.update).await?;

                    let apk_extractor = ApkExtractor::new(server_config.clone())?;
                    apk_extractor.extract_data()?;
                }

                should_process
            }
            ServerRegion::Global => self.handle_global(&apk_fetcher).await?,
        };

        if should_process_catalogs {
            apk_fetcher.check_version().await?;
            self.process_catalogs(&server_config, &apk_fetcher).await?;
        }

        if !should_process_catalogs {
            info!("Catalog files exist and are up to date, skipping catalog processing");
        }

        self.download_resources(&server_config, args).await?;

        Ok(())
    }

    async fn handle_japan(&self, apk_fetcher: &ApkFetcher) -> Result<bool> {
        let data_path = file::get_data_path("data")?;
        let catalog_path = file::get_data_path("catalog")?;

        let data_empty = file::is_dir_empty(&data_path).await?;
        let catalogs_empty = file::is_dir_empty(&catalog_path).await?;

        if data_empty || catalogs_empty || self.args.update {
            return Ok(true);
        }

        apk_fetcher.needs_catalog_update().await
    }

    async fn handle_global(&self, apk_fetcher: &ApkFetcher) -> Result<bool> {
        let catalog_path = file::get_data_path("catalog")?;

        let catalogs_empty = file::is_dir_empty(&catalog_path).await?;

        if catalogs_empty || self.args.update {
            return Ok(true);
        }

        apk_fetcher.needs_catalog_update().await
    }

    async fn process_catalogs(&self, server_config: &Rc<ServerConfig>, apk_fetcher: &ApkFetcher) -> Result<()> {
        let catalog_fetcher = CatalogFetcher::new(server_config.clone(), apk_fetcher.clone())?;
            let catalog_parser = CatalogParser::new(server_config.clone())?;

        catalog_fetcher.get_addressable().await?;
        catalog_fetcher.get_catalogs().await?;
        catalog_parser.process_catalogs().await?;

        Ok(())
    }

    async fn download_resources(&self, server_config: &Rc<ServerConfig>, args: &DownloadArgs) -> Result<()> {
        let resource_downloader = ResourceDownloadBuilder::new(server_config.clone())?
            .limit(args.limit as u64)
            .retries(args.retries)
            .output(Some(args.output.clone().into()))
            .build()
            .await?;

        let resource_category = self.resource_category(args);
        let resource_filter = self.resource_filter(args)?;
        resource_downloader
            .download(resource_category, resource_filter)
            .await?;

        Ok(())
    }

    fn resource_category(&self, args: &DownloadArgs) -> ResourceCategory {
        let has_android_assets = args.android_assets;
		let has_ios_assets = args.ios_assets;
        let has_tables = args.tables;
        let has_media = args.media;

		let mut categories = Vec::new();

		if has_android_assets {
			categories.push(ResourceCategory::AndroidAssets);
		}
		if has_ios_assets {
			categories.push(ResourceCategory::iOSAssets);
		}
		if has_tables {
			categories.push(ResourceCategory::Tables);
		}
		if has_media {
			categories.push(ResourceCategory::Media);
		}

		match categories.len() {
			0 => ResourceCategory::All,
			1 => categories.into_iter().next().unwrap(),
			_ => ResourceCategory::Multiple(categories),
		}
    }

    fn resource_filter(&self, args: &DownloadArgs) -> Result<Option<ResourceFilter>> {
        let Some(filter_pattern) = &args.filter else {
            if !matches!(args.filter_method, FilterMethod::Contains) {
                let filter_method_name = format!("{:?}", args.filter_method).to_lowercase();
                return None.error_context(&format!(
                    "Filter method '{}' specified but no filter pattern provided. Use --filter to specify a pattern.",
                    filter_method_name
                ));
            }
            return Ok(None);
        };

        let filter = ResourceFilter::new(filter_pattern, args.filter_method.clone())?;
        Ok(Some(filter))
    }
}

pub async fn run(args: Args) -> Result<()> {
    let handler = CommandHandler::new(args)?;
    handler.handle().await
}
