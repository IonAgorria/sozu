use anyhow::{self, bail, Context};
use prettytable::Table;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::Serialize;

use sozu_command_lib::{
    command::{
        CommandRequest, CommandRequestOrder, CommandResponse, CommandResponseContent,
        CommandStatus, FrontendFilters, RunState, WorkerInfo,
    },
    proxy::{
        MetricsConfiguration, ProxyRequestOrder, Query, QueryCertificateType, QueryClusterDomain,
        QueryClusterType, QueryMetricsOptions,
    },
};

use crate::{
    cli::MetricsCmd,
    ctl::{
        create_channel,
        display::{
            print_available_metrics, print_certificates, print_frontend_list, print_json_response,
            print_metrics, print_query_response_data, print_status,
        },
        CommandManager,
    },
};

// Used to display the JSON response of the status command
#[derive(Serialize, Debug)]
struct WorkerStatus<'a> {
    pub worker: &'a WorkerInfo,
    pub status: &'a String,
}

fn generate_id() -> String {
    let s: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(|c| c as char)
        .collect();
    format!("ID-{}", s)
}

fn generate_tagged_id(tag: &str) -> String {
    let s: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(|c| c as char)
        .collect();
    format!("{}-{}", tag, s)
}

impl CommandManager {
    fn read_channel_message_with_timeout(&mut self) -> anyhow::Result<CommandResponse> {
        match self
            .channel
            .read_message_blocking_timeout(Some(self.timeout))
        {
            None => bail!("Command timeout. The proxy didn't send an answer"),
            Some(payload) => Ok(payload),
        }
    }

    pub fn save_state(&mut self, path: String) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::SaveState { path },
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent
            }
            CommandStatus::Error => {
                bail!("could not save proxy state: {}", message.message)
            }
            CommandStatus::Ok => {
                println!("{}", message.message);
            }
        }

        Ok(())
    }

    pub fn load_state(&mut self, path: String) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::LoadState { path: path.clone() },
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent
            }
            CommandStatus::Error => {
                bail!("could not load proxy state: {}", message.message)
            }
            CommandStatus::Ok => {
                println!("Proxy state loaded successfully from {}", path);
            }
        }

        Ok(())
    }

    pub fn dump_state(&mut self, json: bool) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::DumpState,
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent
            }
            CommandStatus::Error => {
                if json {
                    print_json_response(&message.message)?;
                }
                bail!("could not dump proxy state: {}", message.message);
            }
            CommandStatus::Ok => {
                if let Some(CommandResponseContent::State(state)) = message.content {
                    if json {
                        print_json_response(&state)?;
                    } else {
                        println!("{:#?}", state);
                    }
                    return Ok(());
                }
                bail!("state dump was empty");
            }
        }
        Ok(())
    }

    pub fn soft_stop(&mut self, proxy_id: Option<u32>) -> Result<(), anyhow::Error> {
        println!("shutting down proxy");
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::SoftStop)),
            proxy_id,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }

        match message.status {
            CommandStatus::Processing => {
                println!("Proxy is processing: {}", message.message);
            }
            CommandStatus::Error => {
                bail!("could not stop the proxy: {}", message.message);
            }
            CommandStatus::Ok => {
                println!("Proxy shut down with message: \"{}\"", message.message);
            }
        }

        Ok(())
    }

    pub fn hard_stop(&mut self, proxy_id: Option<u32>) -> Result<(), anyhow::Error> {
        println!("shutting down proxy");
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::HardStop)),
            proxy_id,
        ));

        let message = self.read_channel_message_with_timeout()?;

        match message.status {
            CommandStatus::Processing => {
                println!("Proxy is processing: {}", message.message);
            }
            CommandStatus::Error => {
                bail!("could not stop the proxy: {}", message.message);
            }
            CommandStatus::Ok => {
                if id == message.id {
                    println!("Proxy shut down: {}", message.message);
                }
            }
        }
        Ok(())
    }

    // 1. Request a list of workers
    // 2. Send an UpgradeMain
    // 3. Send an UpgradeWorker to each worker
    pub fn upgrade_main(&mut self) -> Result<(), anyhow::Error> {
        println!("Preparing to upgrade proxy...");

        let id = generate_tagged_id("LIST-WORKERS");
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::ListWorkers,
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("Error: received unexpected message: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                error!("Error: the proxy didn't return list of workers immediately");
            }
            CommandStatus::Error => {
                bail!(
                    "Error: failed to get the list of worker: {}",
                    message.message
                );
            }
            CommandStatus::Ok => {
                if let Some(CommandResponseContent::Workers(ref workers)) = message.content {
                    let mut table = Table::new();
                    table.set_format(*prettytable::format::consts::FORMAT_BOX_CHARS);
                    table.add_row(row!["Worker", "pid", "run state"]);
                    for worker in workers.iter() {
                        let run_state = format!("{:?}", worker.run_state);
                        table.add_row(row![worker.id, worker.pid, run_state]);
                    }
                    println!();
                    table.printstd();
                    println!();

                    let id = generate_tagged_id("UPGRADE-MAIN");
                    self.channel.write_message(&CommandRequest::new(
                        id.clone(),
                        CommandRequestOrder::UpgradeMain,
                        None,
                    ));
                    println!("Upgrading main process");

                    loop {
                        let message = self.read_channel_message_with_timeout()?;

                        if id != message.id {
                            bail!("Error: received unexpected message: {:?}", message);
                        }

                        match message.status {
                            CommandStatus::Processing => {
                                println!("Main process is upgrading");
                            }
                            CommandStatus::Error => {
                                bail!("Error: failed to upgrade the main: {}", message.message);
                            }
                            CommandStatus::Ok => {
                                println!("Main process upgrade succeeded: {}", message.message);
                                break;
                            }
                        }
                    }

                    // Reconnect to the new main
                    println!("Reconnecting to new main process...");
                    self.channel = create_channel(&self.config)
                        .with_context(|| "could not reconnect to the command unix socket")?;

                    // Do a rolling restart of the workers
                    let running_workers = workers
                        .iter()
                        .filter(|worker| worker.run_state == RunState::Running)
                        .collect::<Vec<_>>();
                    let running_count = running_workers.len();
                    for (i, worker) in running_workers.iter().enumerate() {
                        println!(
                            "Upgrading worker {} (#{} out of {})",
                            worker.id,
                            i + 1,
                            running_count
                        );

                        self.upgrade_worker(worker.id)
                            .with_context(|| "Upgrading the worker failed")?;
                        //thread::sleep(Duration::from_millis(1000));
                    }

                    println!("Proxy successfully upgraded!");
                }
            }
        }
        Ok(())
    }

    pub fn upgrade_worker(&mut self, worker_id: u32) -> Result<(), anyhow::Error> {
        println!("upgrading worker {}", worker_id);
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::UpgradeWorker(worker_id),
            //FIXME: we should be able to soft stop one specific worker
            None,
        ));

        let message = self
            .channel
            .read_message_blocking_timeout(Some(self.timeout));

        match message {
            None => bail!(format!(
                "No response from the proxy about worker {}",
                worker_id
            )),
            Some(message) => match message.status {
                CommandStatus::Processing => {
                    info!("Worker {} is processing: {}", worker_id, message.message);
                }
                CommandStatus::Error => bail!(
                    "could not stop the worker {}: {}",
                    worker_id,
                    message.message
                ),
                CommandStatus::Ok => {
                    if id == message.id {
                        info!("Worker {} shut down: {}", worker_id, message.message);
                    }
                }
            },
        }
        Ok(())
    }

    pub fn status(&mut self, json: bool) -> anyhow::Result<()> {
        let id = generate_id();

        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::Status,
            None,
        ));

        let message = self
            .channel
            .read_message_blocking_timeout(Some(self.timeout))
            .with_context(|| "Can not read response from channel")?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }

        match message.status {
            CommandStatus::Processing => {
                bail!("should have obtained an answer immediately");
            }
            CommandStatus::Error => {
                if json {
                    print_json_response(&message.message)?;
                }
                bail!("could not get the worker list: {}", message.message);
            }
            CommandStatus::Ok => match message.content {
                Some(CommandResponseContent::Status(worker_info_vec)) => {
                    print_status(worker_info_vec);
                }
                Some(_) => {
                    bail!("Received the wrong kind of response data from the command server")
                }
                None => bail!("No data in the response"),
            },
        }
        Ok(())
    }

    pub fn configure_metrics(&mut self, cmd: MetricsCmd) -> Result<(), anyhow::Error> {
        let id = generate_id();
        //println!("will send message for metrics with id {}", id);

        let configuration = match cmd {
            MetricsCmd::Enable => MetricsConfiguration::Enabled(true),
            MetricsCmd::Disable => MetricsConfiguration::Enabled(false),
            MetricsCmd::Clear => MetricsConfiguration::Clear,
            _ => bail!("The command passed to the configure_metrics function is wrong."),
        };

        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::ConfigureMetrics(
                configuration,
            ))),
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        match message.status {
            CommandStatus::Processing => {
                println!("Proxy is processing: {}", message.message);
            }
            CommandStatus::Error => {
                bail!("Error with metrics command: {}", message.message);
            }
            CommandStatus::Ok => {
                if id == message.id {
                    println!("Successfull metrics command: {}", message.message);
                }
            }
        }
        Ok(())
    }

    pub fn get_metrics(
        &mut self,
        json: bool,
        list: bool,
        refresh: Option<u32>,
        metric_names: Vec<String>,
        cluster_ids: Vec<String>,
        backend_ids: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        let command = CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::Query(
            Query::Metrics(QueryMetricsOptions {
                list,
                cluster_ids,
                backend_ids,
                metric_names,
            }),
        )));

        // a loop to reperform the query every refresh time
        loop {
            let id = generate_id();
            self.channel
                .write_message(&CommandRequest::new(id.clone(), command.clone(), None));
            print!("{}", termion::cursor::Save);

            // this functions may bail and escape the loop, should we avoid that?
            let message = self.read_channel_message_with_timeout()?;

            //println!("received message: {:?}", message);
            if id != message.id {
                bail!("received message with invalid id: {:?}", message);
            }
            match message.status {
                CommandStatus::Processing => {
                    // do nothing here
                    // for other messages, we would loop over read_message
                    // until an error or ok message was sent
                }
                CommandStatus::Error => {
                    if json {
                        return print_json_response(&message.message);
                    } else {
                        bail!("could not query proxy state: {}", message.message);
                    }
                }
                CommandStatus::Ok => match message.content {
                    Some(CommandResponseContent::Metrics(aggregated_metrics_data)) => {
                        print_metrics(aggregated_metrics_data, json)?
                    }
                    Some(CommandResponseContent::Query(lists_of_metrics)) => {
                        print_available_metrics(&lists_of_metrics)?;
                    }
                    _ => println!("Wrong kind of response here"),
                },
            }

            match refresh {
                None => break,
                Some(seconds) => std::thread::sleep(std::time::Duration::from_secs(seconds as u64)),
            }

            print!(
                "{}{}",
                termion::cursor::Restore,
                termion::clear::BeforeCursor
            );
        }

        Ok(())
    }

    pub fn reload_configuration(
        &mut self,
        path: Option<String>,
        json: bool,
    ) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::ReloadConfiguration { path },
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                bail!("should have obtained an answer immediately");
            }
            CommandStatus::Error => {
                if json {
                    print_json_response(&message.message)?;
                }
                bail!("could not get the worker list: {}", message.message);
            }
            CommandStatus::Ok => {
                if json {
                    print_json_response(&message.message)?;
                } else {
                    println!("Reloaded configuration: {}", message.message);
                }
            }
        }

        Ok(())
    }

    pub fn list_frontends(
        &mut self,
        http: bool,
        https: bool,
        tcp: bool,
        domain: Option<String>,
    ) -> Result<(), anyhow::Error> {
        let command = CommandRequestOrder::ListFrontends(FrontendFilters {
            http,
            https,
            tcp,
            domain,
        });

        let id = generate_id();
        self.channel
            .write_message(&CommandRequest::new(id.clone(), command, None));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {}
            CommandStatus::Error => {
                println!("could not query proxy state: {}", message.message)
            }
            CommandStatus::Ok => {
                debug!("We received this response: {:?}", message.content);

                if let Some(CommandResponseContent::FrontendList(frontends)) = message.content {
                    print_frontend_list(frontends);
                }
            }
        }

        Ok(())
    }

    pub fn query_cluster(
        &mut self,
        json: bool,
        cluster_id: Option<String>,
        domain: Option<String>,
    ) -> Result<(), anyhow::Error> {
        if cluster_id.is_some() && domain.is_some() {
            bail!("Error: Either request an cluster ID or a domain name");
        }

        let command = if let Some(ref cluster_id) = cluster_id {
            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::Query(Query::Clusters(
                QueryClusterType::ClusterId(cluster_id.to_string()),
            ))))
        } else if let Some(ref domain) = domain {
            let splitted: Vec<String> =
                domain.splitn(2, '/').map(|elem| elem.to_string()).collect();

            if splitted.is_empty() {
                bail!("Domain can't be empty");
            }

            let query_domain = QueryClusterDomain {
                hostname: splitted
                    .get(0)
                    .with_context(|| "Domain can't be empty")?
                    .clone(),
                path: splitted.get(1).cloned().map(|path| format!("/{}", path)), // We add the / again because of the splitn removing it
            };

            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::Query(Query::Clusters(
                QueryClusterType::Domain(query_domain),
            ))))
        } else {
            CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::Query(Query::ClustersHashes)))
        };

        let id = generate_id();
        self.channel
            .write_message(&CommandRequest::new(id.clone(), command, None));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent

                // or maybe just print what the processing has to say?
                println!("{}", message.message);
            }
            CommandStatus::Error => {
                if json {
                    print_json_response(&message.message)?;
                }
                bail!("could not query proxy state: {}", message.message);
            }
            CommandStatus::Ok => {
                print_query_response_data(cluster_id, domain, message.content, json)?;
            }
        }

        Ok(())
    }

    pub fn query_certificate(
        &mut self,
        json: bool,
        fingerprint: Option<String>,
        domain: Option<String>,
    ) -> Result<(), anyhow::Error> {
        let query = match (fingerprint, domain) {
            (None, None) => QueryCertificateType::All,
            (Some(f), None) => match hex::decode(f) {
                Err(e) => {
                    bail!("invalid fingerprint: {:?}", e);
                }
                Ok(f) => QueryCertificateType::Fingerprint(f),
            },
            (None, Some(d)) => QueryCertificateType::Domain(d),
            (Some(_), Some(_)) => {
                bail!("Error: Either request a fingerprint or a domain name");
            }
        };

        let command = CommandRequestOrder::Proxy(Box::new(ProxyRequestOrder::Query(
            Query::Certificates(query),
        )));

        let id = generate_id();
        self.channel
            .write_message(&CommandRequest::new(id.clone(), command, None));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent
            }
            CommandStatus::Error => {
                if json {
                    print_json_response(&message.message)?;
                    bail!("We received an error message");
                } else {
                    bail!("could not query proxy state: {}", message.message);
                }
            }
            CommandStatus::Ok => {
                if let Some(CommandResponseContent::Query(data)) = message.content {
                    print_certificates(data, json)?;
                } else {
                    bail!("unexpected response: {:?}", message.content);
                }
            }
        }
        Ok(())
    }

    pub fn events(&mut self) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id,
            CommandRequestOrder::SubscribeEvents,
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;
        match message.status {
            CommandStatus::Processing => {
                if let Some(CommandResponseContent::Event(event)) = message.content {
                    println!("got event from worker({}): {:?}", message.message, event);
                }
            }
            CommandStatus::Error => {
                bail!("could not get proxy events: {}", message.message);
            }
            CommandStatus::Ok => {
                println!("{}", message.message);
            }
        }
        Ok(())
    }

    pub fn order_command(&mut self, order: ProxyRequestOrder) -> Result<(), anyhow::Error> {
        let id = generate_id();
        self.channel.write_message(&CommandRequest::new(
            id.clone(),
            CommandRequestOrder::Proxy(Box::new(order)),
            None,
        ));

        let message = self.read_channel_message_with_timeout()?;

        if id != message.id {
            bail!("received message with invalid id: {:?}", message);
        }
        match message.status {
            CommandStatus::Processing => {
                // do nothing here
                // for other messages, we would loop over read_message
                // until an error or ok message was sent
            }
            CommandStatus::Error => bail!("could not execute order: {}", message.message),
            CommandStatus::Ok => {
                //deactivate success messages for now
                /*
                match order {
                  ProxyRequestOrder::AddCluster(_) => println!("cluster added : {}", message.message),
                  ProxyRequestOrder::RemoveCluster(_) => println!("cluster removed : {} ", message.message),
                  ProxyRequestOrder::AddBackend(_) => println!("backend added : {}", message.message),
                  ProxyRequestOrder::RemoveBackend(_) => println!("backend removed : {} ", message.message),
                  ProxyRequestOrder::AddCertificate(_) => println!("certificate added: {}", message.message),
                  ProxyRequestOrder::RemoveCertificate(_) => println!("certificate removed: {}", message.message),
                  ProxyRequestOrder::AddHttpFrontend(_) => println!("front added: {}", message.message),
                  ProxyRequestOrder::RemoveHttpFrontend(_) => println!("front removed: {}", message.message),
                  _ => {
                    // do nothing for now
                  }
                }
                */
            }
        }
        Ok(())
    }
}