// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
};

use rand::Rng;
use slim_datapath::messages::{Agent, AgentType};

use clap::Parser;
use indicatif::ProgressBar;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Number of subscriptions to produce
    #[arg(short, long, value_name = "SUBSCRIPTIONS", required = true)]
    subscriptions: u32,

    /// Number of publications to produce
    #[arg(short, long, value_name = "PUBLICATIONS", required = true)]
    publications: u32,

    /// Maximum number of agent instances for the same type
    #[arg(
        short,
        long,
        value_name = "INSTANCES",
        required = false,
        default_value_t = 1
    )]
    instances: u32,

    /// Number of subscriber agents
    #[arg(
        short,
        long,
        value_name = "AGENTS",
        required = false,
        default_value_t = 1
    )]
    agents: u32,

    /// Output file where to store the workload
    #[arg(
        short,
        long,
        value_name = "OUTPUT",
        required = false,
        default_value = "autogenerated"
    )]
    output: String,
}

impl Args {
    pub fn subscriptions(&self) -> &u32 {
        &self.subscriptions
    }

    pub fn publications(&self) -> &u32 {
        &self.publications
    }

    pub fn instances(&self) -> &u32 {
        &self.instances
    }

    pub fn agents(&self) -> &u32 {
        &self.agents
    }

    pub fn output(&mut self) -> &String {
        if self.output == "autogenerated" {
            self.output = "sub".to_string();
            self.output.push_str(&self.subscriptions().to_string());
            self.output.push_str("_pub");
            self.output.push_str(&self.publications().to_string());
            self.output.push_str("_i");
            self.output.push_str(&self.instances().to_string());
            self.output.push_str("_s");
            self.output.push_str(&self.agents().to_string());
            self.output.push_str(".dat");
        }

        &self.output
    }
}

#[derive(Default, Debug)]
struct SubscriptionPair {
    /// agent id to which the subscriber subscribes
    agent_id: u64,
    /// subscriber id
    subscriber: u32,
}

#[derive(Default, Debug)]
struct TypeState {
    /// list of the ids of the scribscribers of this type
    subscribers: HashSet<u32>,
    /// list of pairs agnet_id, subscriber
    subscription: Vec<SubscriptionPair>,
}

impl TypeState {
    fn insert(&mut self, agent_id: u64, subscriber: u32) {
        self.subscribers.insert(subscriber);
        let pair = SubscriptionPair {
            agent_id,
            subscriber,
        };
        self.subscription.push(pair);
    }

    fn get_pair(&self) -> &SubscriptionPair {
        let mut rng = rand::rng();
        let index = rng.random_range(..self.subscription.len());
        &self.subscription[index]
    }
}

struct Publication {
    /// publication name
    publication: Agent,
    /// list of ids that can receive the publication
    subscribers: HashSet<u32>,
}

fn main() {
    // parse command line arguments
    let mut args = Args::parse();

    let max_subscriptions = *args.subscriptions();
    let max_publications = *args.publications();
    let max_agents_per_type = *args.instances();
    let subscribers = *args.agents();
    let output = args.output();

    println!(
        "configuration -- subscriptions: {}, publications: {}, agents per type: {}, agents: {}, output: {}",
        max_subscriptions, max_publications, max_agents_per_type, subscribers, output
    );

    // number of subscription created
    let mut subscriptions = 0;

    let mut rng = rand::rng();

    let mut subscription_list = HashMap::new();
    let mut publications_list = Vec::new();

    // crete max_subscription subscriptions
    println!("creating subscriptions");
    let bar = ProgressBar::new(max_subscriptions as u64);
    while subscriptions < max_subscriptions {
        // create a new type
        let org = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(7)
            .map(char::from)
            .collect::<String>();

        let ns = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(7)
            .map(char::from)
            .collect::<String>();

        let atype = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(7)
            .map(char::from)
            .collect::<String>();

        let agent_type = AgentType::from_strings(&org, &ns, &atype);

        let mut type_state = TypeState::default();

        // for each type create at most max_agent_per_type
        // but at least one
        let mut agent_per_type = 0;
        let mut add_agent = true;

        // stop if:
        // 1) the nunmber of agents ids is equal to max_agent_per_type in this type
        // 2) the random bool add_agent is false
        // 3) the total number of subscrptions is equal to max_subscription
        while (agent_per_type < max_agents_per_type)
            && add_agent
            && (subscriptions < max_subscriptions)
        {
            // the agent id in the subscription is always a random numb
            let sub = Agent::new(agent_type.clone(), rng.random_range(0..u64::MAX));

            // decide which agent will send the subscription
            let subscriber = rng.random_range(..subscribers);
            agent_per_type += 1;
            subscriptions += 1;
            add_agent = rng.random_bool(0.8);

            bar.inc(1);

            type_state.insert(sub.agent_id(), subscriber);
        }

        subscription_list.insert(agent_type, type_state);
    }
    bar.finish();

    // number of publications created
    let mut publications = 0;

    // crete max_publications publications
    // iterate over the map multiple times if needed
    println!("creating publications");
    let bar = ProgressBar::new(max_publications as u64);
    while publications < max_publications {
        for s in subscription_list.iter() {
            // randomly decided to use the agent id or not
            let use_agent_id = rng.random_bool(0.5);
            if use_agent_id {
                // pick a random pair for this type
                let pair = s.1.get_pair();

                let name = Agent::new(s.0.clone(), pair.agent_id);

                let mut p = Publication {
                    publication: name,
                    subscribers: HashSet::new(),
                };
                p.subscribers.insert(pair.subscriber);

                publications_list.push(p);
            } else {
                let name = Agent::new(s.0.clone(), 0);
                let mut p = Publication {
                    publication: name,
                    subscribers: HashSet::new(),
                };

                for v in s.1.subscribers.iter() {
                    p.subscribers.insert(*v);
                }

                publications_list.push(p);
            }

            // stop if enough publications are created
            publications += 1;

            bar.inc(1);
            if publications >= max_publications {
                break;
            }
        }
    }
    bar.finish();

    let res = File::create(output);
    if res.is_err() {
        panic!("error creating the file");
    }

    let mut file = res.unwrap();
    // output
    let mut i = 0;
    // print subscriptions
    println!("writing subscriptions to file");
    let bar = ProgressBar::new(max_subscriptions as u64);
    for s in subscription_list.iter() {
        for p in s.1.subscription.iter() {
            // format: SUB index subscriber org ns type id
            let s = format!(
                "SUB {} {} {} {} {} {}\n",
                i,
                p.subscriber,
                s.0.organization(),
                s.0.namespace(),
                s.0.agent_type(),
                p.agent_id
            );
            let res = file.write_all(s.as_bytes());
            if res.is_err() {
                panic!("error writing to the file");
            }
            i += 1;
            bar.inc(1);
        }
    }
    bar.finish();

    i = 0;
    println!("writing publications to file");
    let bar = ProgressBar::new(max_publications as u64);
    for m in publications_list.iter() {
        let mut sub_str = String::new();
        for a in m.subscribers.iter() {
            sub_str.push(' ');
            sub_str.push_str(&a.to_string());
        }
        // format: PUB index org ns type id #subscribers subscriber_1, subscriber_2, ...
        let s = format!(
            "PUB {} {} {} {} {} {}{}\n", //do not add the space between m.subscribers.len() and sub_str
            i,
            m.publication.agent_type().organization(),
            m.publication.agent_type().namespace(),
            m.publication.agent_type().agent_type(),
            m.publication.agent_id(),
            m.subscribers.len(),
            sub_str
        );
        let res = file.write_all(s.as_bytes());
        if res.is_err() {
            panic!("error writing to the file");
        }
        i += 1;
        bar.inc(1);
    }
    bar.finish();

    println!("workload generated")
}
