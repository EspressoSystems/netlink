// SPDX-License-Identifier: MIT

use std::{env, fs::File, path::Path, os::unix::prelude::AsRawFd, net::Ipv4Addr};
use nix::sched::{setns, CloneFlags};
use std::os::unix::io::IntoRawFd;

use futures::TryStreamExt;
use rtnetlink::{new_connection, TcNetemQopt, LinkAddRequest, NetworkNamespace, SELF_NS_PATH, NETNS_PATH, AddressHandle, RouteHandle};

const TEST_DUMMY: &str = "test_dummy";

struct Netns {
    path: String,
    cur: File,
    last: File,
}
impl Netns {
    async fn new(path: &str) -> Self {
        // record current ns
        let last = File::open(Path::new(SELF_NS_PATH)).unwrap();

        // create new ns
        NetworkNamespace::add(path.to_string()).await.unwrap();

        // entry new ns
        let ns_path = Path::new(NETNS_PATH);
        let file = File::open(ns_path.join(path)).unwrap();

        Self {
            path: path.to_string(),
            cur: file,
            last,
        }
    }
}
impl Drop for Netns {
    fn drop(&mut self) {
        // setns(self.last.as_raw_fd(), CloneFlags::CLONE_NEWNET).unwrap();
        //
        // let ns_path = Path::new(NETNS_PATH).join(&self.path);
        // nix::mount::umount2(&ns_path, nix::mount::MntFlags::MNT_DETACH).unwrap();
        // nix::unistd::unlink(&ns_path).unwrap();
        

        // _cur File will be closed auto
        // Since there is no async drop, NetworkNamespace::del cannot be called
        // here. Dummy interface will be deleted automatically after netns is
        // deleted.
    }
}

#[tokio::main]
async fn main() -> Result<(), ()> {
    env_logger::init();


    let (connection, handle, _) = new_connection().unwrap();
    tokio::spawn(connection);


    // create new netns
    let counter_ns_name = "COUNTER_NS";
    let counter_ns = Netns::new(counter_ns_name).await;


    // create veth interfaces
    let veth = "veth".to_string();
    let veth_2 = "veth2".to_string();

    handle.link().add().veth(veth.clone(), veth_2.clone()).execute().await.map_err(|e| format!("{}", e)).unwrap();
    let veth_idx = handle.link().get().match_name(veth.clone()).execute().try_next().await.unwrap().unwrap().header.index;
    let veth_2_idx = handle.link().get().match_name(veth_2.clone()).execute().try_next().await.unwrap().unwrap().header.index;

    // set interfaces up
    handle.link().set(veth_idx).up().execute().await.unwrap();
    handle.link().set(veth_2_idx).up().execute().await.unwrap();

    // move veth_2 to counter_ns
    handle.link().set(veth_2_idx).setns_by_fd(counter_ns.cur.as_raw_fd()).execute().await.unwrap();

    let bridge_name = "br0".to_string();

    handle.link().add().bridge(bridge_name.clone()).execute().await.unwrap();
    let bridge_idx = handle.link().get().match_name(bridge_name.clone()).execute().try_next().await.unwrap().unwrap().header.index;

    // set bridge up
    handle.link().set(bridge_idx).up().execute().await.unwrap();

    // set veth master to bridge
    handle.link().set(veth_idx).master(bridge_idx).execute().await.unwrap();

    // add ip address to bridge
    let bridge_addr = Ipv4Addr::new(172, 18, 0, 1);
    let bridge_range = 16;
    AddressHandle::new(handle).add(bridge_idx, std::net::IpAddr::V4(bridge_addr), bridge_range).execute().await.unwrap();

    // switch to counter_ns
    setns(counter_ns.cur.as_raw_fd(), CloneFlags::CLONE_NEWNET).unwrap();

    // get connection metadata in new net namespace
    let (connection, handle, _) = new_connection().unwrap();
    tokio::spawn(connection);


    // set lo interface to up
    let lo_idx = handle.link().get().match_name("lo".to_string()).execute().try_next().await.unwrap().unwrap().header.index;
    handle.link().set(lo_idx).up().execute().await.unwrap();

    // set veth2 to up
    let veth_2_idx = handle.link().get().match_name(veth_2).execute().try_next().await.unwrap().unwrap().header.index;
    handle.link().set(veth_2_idx).up().execute().await.unwrap();

    // set veth2 address
    let veth_2_addr = std::net::IpAddr::V4(Ipv4Addr::new(172, 18, 0, 2));
    let veth_2_range = 16;
    AddressHandle::new(handle.clone()).add(veth_2_idx, veth_2_addr, veth_2_range).execute().await.unwrap();

    // add route
    let route = RouteHandle::new(handle).add();
    route.v4().gateway(bridge_addr).execute().await.unwrap();


    drop(counter_ns);
    Ok(())

}

fn usage() {
    eprintln!(
        "usage:
    cargo run --example add_tc_qdisc_ingress -- <index>

Note that you need to run this program as root. Instead of running cargo as root,
build the example normally:

    cd rtnetlink ; cargo build --example add_tc_qdisc_ingress 

Then find the binary in the target directory:

    cd ../target/debug/example ; sudo ./add_tc_qdisc_ingress <index>"
    );
}
