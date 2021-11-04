// SPDX-FileCopyrightText: © 2021 ChiselStrike <info@chiselstrike.com>

use anyhow::{anyhow, Context, Result};
use chisel::chisel_rpc_client::ChiselRpcClient;
use chisel::{
    AddTypeRequest, EndPointCreationRequest, FieldDefinition, PolicyUpdateRequest,
    RemoveTypeRequest, RestartRequest, StatusRequest, TypeExportRequest,
};
use graphql_parser::schema::{parse_schema, Definition, TypeDefinition};
use serde_derive::Deserialize;
use std::fs;
use std::io::{stdin, Read};
use std::path::Path;
use std::thread;
use std::time::Duration;
use structopt::StructOpt;

/// Manifest defines the files that describe types, endpoints, and policies.
#[derive(Deserialize)]
struct Manifest {
    types: Vec<String>,
    endpoints: Vec<String>,
    policies: Vec<String>,
}

impl Manifest {
    pub fn new(types: Vec<String>, endpoints: Vec<String>, policies: Vec<String>) -> Self {
        Manifest {
            types,
            endpoints,
            policies,
        }
    }

    pub fn types(&self) -> Result<Vec<std::path::PathBuf>, anyhow::Error> {
        Self::dirs_to_paths(&self.types)
    }

    pub fn endpoints(&self) -> Result<Vec<std::path::PathBuf>, anyhow::Error> {
        Self::dirs_to_paths(&self.endpoints)
    }

    pub fn policies(&self) -> Result<Vec<std::path::PathBuf>, anyhow::Error> {
        Self::dirs_to_paths(&self.policies)
    }

    fn dirs_to_paths(dirs: &[String]) -> Result<Vec<std::path::PathBuf>, anyhow::Error> {
        let mut paths = vec![];
        for dir in dirs {
            for dentry in read_dir(dir)? {
                let dentry = dentry?;
                paths.push(dentry.path());
            }
        }
        Ok(paths)
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "chisel")]
struct Opt {
    /// RPC server address.
    #[structopt(short, long, default_value = "http://localhost:50051")]
    rpc_addr: String,
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    /// Shows information about ChiselStrike server status.
    Status,
    Type {
        #[structopt(subcommand)]
        cmd: TypeCommand,
    },
    EndPoint {
        #[structopt(subcommand)]
        cmd: EndPointCommand,
    },
    Restart,
    Wait,
    Policy {
        #[structopt(subcommand)]
        cmd: PolicyCommand,
    },
    Apply,
}

#[derive(StructOpt, Debug)]
enum TypeCommand {
    /// Add a type to the type system.
    Add { type_name: String },
    /// Import types to the type system.
    Import {
        /// Type definition input file.
        filename: String,
    },
    /// Export the type system.
    Export,
    /// Remove type from the type system.
    Remove { type_name: String },
}

#[derive(StructOpt, Debug)]
enum EndPointCommand {
    Create { path: String, filename: String },
}

#[derive(StructOpt, Debug)]
enum PolicyCommand {
    /// Must be "transformation ( @label )", where transformation is a known transform function.
    Update { filename: String },
}

pub mod chisel {
    tonic::include_proto!("chisel");
}

/// Opens and reads an entire file (or stdin, if filename is "-")
fn read_to_string<P: AsRef<Path>>(filename: P) -> Result<String, std::io::Error> {
    if filename.as_ref() == Path::new("-") {
        let mut s = "".to_string();
        stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        fs::read_to_string(filename.as_ref())
    }
}

fn read_dir<P: AsRef<Path>>(dir: P) -> Result<fs::ReadDir, anyhow::Error> {
    fs::read_dir(dir.as_ref()).with_context(|| format!("Could not open {}", dir.as_ref().display()))
}

async fn import_types<P>(
    client: &mut ChiselRpcClient<tonic::transport::Channel>,
    filename: P,
) -> Result<()>
where
    P: AsRef<Path>,
{
    let schema = read_to_string(filename)?;
    let type_system = parse_schema::<String>(&schema)?;
    for def in &type_system.definitions {
        match def {
            Definition::TypeDefinition(TypeDefinition::Object(obj_def)) => {
                let mut field_defs = Vec::default();
                for field_def in &obj_def.fields {
                    field_defs.push(FieldDefinition {
                        name: field_def.name.to_owned(),
                        field_type: format!("{}", field_def.field_type.to_owned()),
                        labels: field_def
                            .directives
                            .iter()
                            .map(|d| d.name.clone())
                            .collect(),
                    });
                }
                let request = tonic::Request::new(AddTypeRequest {
                    name: obj_def.name.to_owned(),
                    field_defs,
                });
                let response = client.add_type(request).await?.into_inner();
                println!("Type defined: {}", response.message);
            }
            def => {
                println!("Ignoring type definition: {:?}", def);
            }
        }
    }
    Ok(())
}

async fn create_endpoint<P>(
    client: &mut ChiselRpcClient<tonic::transport::Channel>,
    path: String,
    filename: P,
) -> Result<()>
where
    P: AsRef<Path>,
{
    let code = read_to_string(&filename)?;
    let request = tonic::Request::new(EndPointCreationRequest { path, code });
    let response = client.create_end_point(request).await?.into_inner();
    println!("End point defined: {}", response.message);
    Ok(())
}

async fn wait(server_url: String) {
    let mut wait_time = 1;
    loop {
        let mut client = match ChiselRpcClient::connect(server_url.clone()).await {
            Ok(client) => client,
            Err(_) => {
                thread::sleep(Duration::from_secs(wait_time));
                wait_time *= 2;
                continue;
            }
        };
        let request = tonic::Request::new(StatusRequest {});
        match client.get_status(request).await {
            Ok(_) => break,
            Err(_) => {
                thread::sleep(Duration::from_secs(wait_time));
                wait_time *= 2;
                continue;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();
    let server_url = opt.rpc_addr;
    match opt.cmd {
        Command::Status => {
            let mut client = ChiselRpcClient::connect(server_url).await?;
            let request = tonic::Request::new(StatusRequest {});
            let response = client.get_status(request).await?.into_inner();
            println!("Server status is {}", response.message);
        }
        Command::EndPoint { cmd } => match cmd {
            EndPointCommand::Create { path, filename } => {
                let mut client = ChiselRpcClient::connect(server_url).await?;
                create_endpoint(&mut client, path, filename).await?
            }
        },
        Command::Type { cmd } => match cmd {
            TypeCommand::Add { type_name } => {
                let mut client = ChiselRpcClient::connect(server_url).await?;
                let request = tonic::Request::new(AddTypeRequest {
                    name: type_name,
                    field_defs: vec![],
                });
                let response = client.add_type(request).await?.into_inner();
                println!("Type defined: {}", response.message);
            }
            TypeCommand::Import { filename } => {
                let mut client = ChiselRpcClient::connect(server_url).await?;
                import_types(&mut client, filename).await?;
            }
            TypeCommand::Export => {
                let mut client = ChiselRpcClient::connect(server_url).await?;
                let request = tonic::Request::new(TypeExportRequest {});
                let response = client.export_types(request).await?.into_inner();
                for def in response.type_defs {
                    println!("class {} {{", def.name);
                    for field in def.field_defs {
                        println!(
                            "  {}: {}{}",
                            field.name,
                            field.field_type,
                            field
                                .labels
                                .iter()
                                .map(|x| format!(" @{}", x))
                                .collect::<String>()
                        );
                    }
                    println!("}}");
                }
            }
            TypeCommand::Remove { type_name } => {
                let mut client = ChiselRpcClient::connect(server_url).await?;
                let request = tonic::Request::new(RemoveTypeRequest {
                    type_name: type_name.clone(),
                });
                let _ = client.remove_type(request).await?.into_inner();
                println!("Type removed: {}", type_name);
            }
        },
        Command::Restart => {
            let mut client = ChiselRpcClient::connect(server_url.clone()).await?;
            let response = client
                .restart(tonic::Request::new(RestartRequest {}))
                .await?
                .into_inner();
            println!("{}", if response.ok { "success" } else { "failure" });
            wait(server_url).await
        }
        Command::Wait => wait(server_url).await,
        Command::Policy { cmd } => match cmd {
            PolicyCommand::Update { filename } => {
                let policystr = read_to_string(&filename)?;

                let response = ChiselRpcClient::connect(server_url)
                    .await?
                    .policy_update(tonic::Request::new(PolicyUpdateRequest {
                        policy_config: policystr,
                    }))
                    .await?
                    .into_inner();
                println!("Policy updated: {}", response.message);
            }
        },
        Command::Apply => {
            let manifest = match read_to_string("Chisel.toml") {
                Ok(manifest) => toml::from_str(&manifest)?,
                _ => Manifest::new(
                    vec!["./types".to_string()],
                    vec!["./endpoints".to_string()],
                    vec!["./policies".to_string()],
                ),
            };
            let types = manifest.types()?;
            let endpoints = manifest.endpoints()?;
            let policies = manifest.policies()?;

            let mut client = ChiselRpcClient::connect(server_url).await?;

            for entry in types {
                // FIXME: will fail the second time until we implement type evolution
                import_types(&mut client, entry).await?;
            }

            for entry in endpoints {
                // FIXME: disambiguate endpoint and endpoint.js. If you have both, this has to
                // error out. For now simplify.
                let endpoint_name = entry
                    .file_stem()
                    .ok_or_else(|| anyhow!("Invalid endpoint filename {:?}", entry))?
                    .to_os_string()
                    .into_string()
                    .unwrap();

                create_endpoint(&mut client, endpoint_name, entry).await?;
            }

            for entry in policies {
                let policystr = read_to_string(entry)?;

                let response = client
                    .policy_update(tonic::Request::new(PolicyUpdateRequest {
                        policy_config: policystr,
                    }))
                    .await?
                    .into_inner();
                println!("Policy applied: {}", response.message);
            }
        }
    }
    Ok(())
}
