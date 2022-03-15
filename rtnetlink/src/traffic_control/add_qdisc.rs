// SPDX-License-Identifier: MIT

use futures::stream::StreamExt;
use netlink_packet_route::{tc::TcOpt, nlas::{DefaultNla, NlaBuffer}, TCA_OPTIONS, traits::Parseable, TCA_CHAIN, TCA_NETEM_CORR, TCA_NETEM_REORDER, TCA_NETEM_CORRUPT, TCA_NETEM_LOSS, DecodeError};
use byteorder::{NativeEndian, ByteOrder};

use crate::{
    packet::{
        tc::{constants::*, nlas},
        NetlinkMessage,
        RtnlMessage,
        TcMessage,
        NLM_F_ACK,
        NLM_F_REQUEST,
        TC_H_MAKE,
    },
    try_nl,
    Error,
    Handle,
};

/// convert from percentage (value 0-100) to value
/// expected by nla buffer
#[inline(always)]
fn percent_to_buffer(p: u32) -> u32 {
    ((1 << 31) / 50) * p
}


#[derive(Clone, Debug)]
pub struct NetemQdisc {
    pub config: TcNetemQopt,
    pub delay: Option<TcNetemDelay>,
    pub correlations: Option<TcNetemCorrelations>,
    pub corruption: Option<TcNetemCorrupt>,
    pub reorder: Option<TcNetemReorder>,
}

impl Default for NetemQdisc {
    fn default() -> Self {
        Self { config: Default::default(), delay: Some(Default::default()), correlations: Some(Default::default()), corruption: Some(Default::default()), reorder: Some(Default::default()), }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TcNetemQopt {
    /// limit of number of packets as u32
    pub limit: u32,
    /// loss as percentage
    pub loss: u32,
    /// gap between packets
    pub gap: u32,
    /// duplicate packets as %
    pub duplicate: u32,
}

impl Default for TcNetemQopt {
    fn default() -> Self {
        Self { limit: 10240, loss: 0, gap: 0, duplicate: 0 }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TcNetemDelay {
    pub delay: u64,
    pub stddev: u64,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TcNetemCorrelations {
    pub delay_corr: u32,
    pub loss_corr: u32,
    pub dup_corr: u32,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TcNetemReorder {
    pub prob: u32,
    pub corr: u32,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TcNetemCorrupt {
    pub prob: u32,
    pub corr: u32,
}

pub struct QDiscNewRequest {
    handle: Handle,
    message: TcMessage,
    flags: u16,
}

impl QDiscNewRequest {
    pub(crate) fn new(handle: Handle, message: TcMessage, flags: u16) -> Self {
        Self {
            handle,
            message,
            flags: NLM_F_REQUEST | flags,
        }
    }

    /// Execute the request
    pub async fn execute(self) -> Result<(), Error> {
        let Self {
            mut handle,
            message,
            flags,
        } = self;

        let mut req = NetlinkMessage::from(RtnlMessage::NewQueueDiscipline(message));
        req.header.flags = NLM_F_ACK | flags;
        req.finalize();

        let mut response = handle.request(req)?;
        while let Some(message) = response.next().await {
            try_nl!(message);
        }
        Ok(())
    }

    /// Set handle,
    pub fn handle(mut self, maj: u16, min: u16) -> Self {
        self.message.header.handle = TC_H_MAKE!((maj as u32) << 16, min as u32);
        self
    }

    /// Set parent to root.
    pub fn root(mut self) -> Self {
        assert_eq!(self.message.header.parent, TC_H_UNSPEC);
        self.message.header.parent = TC_H_ROOT;
        self
    }

    /// Set parent
    pub fn parent(mut self, parent: u32) -> Self {
        assert_eq!(self.message.header.parent, TC_H_UNSPEC);
        self.message.header.parent = parent;
        self
    }

    /// New a ingress qdisc
    pub fn ingress(mut self) -> Self {
        assert_eq!(self.message.header.parent, TC_H_UNSPEC);
        self.message.header.parent = TC_H_INGRESS;
        self.message.header.handle = 0xffff0000;
        self.message
            .nlas
            .push(nlas::Nla::Kind("ingress".to_string()));
        self
    }

    /// Create a new netem qdisc
    pub fn netem(mut self, opts: NetemQdisc) -> Result<Self, DecodeError> {
        let mut nlas = Vec::new();
        assert_eq!(self.message.header.parent, TC_H_UNSPEC);
        self.message.header.parent = TC_H_ROOT;
        self.message
            .nlas
            .push(nlas::Nla::Kind("netem".to_string()));

        let mut raw_buf: Vec<u8> = vec![0; 24];
        NativeEndian::write_u16(&mut raw_buf[0..2], 24);
        // limit
        NativeEndian::write_u32(&mut raw_buf[4..8], opts.config.limit);
        // loss %
        NativeEndian::write_u32(&mut raw_buf[8..12], ((1 << 31) / 50) * opts.config.loss);
        // gap (as u32)
        NativeEndian::write_u32(&mut raw_buf[12..16], opts.config.gap);
        // duplicate
        NativeEndian::write_u32(&mut raw_buf[16..20], ((1 << 31) / 50) * opts.config.duplicate);
        // irrelevant field
        NativeEndian::write_u32(&mut raw_buf[20..24], 0);
        let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
        buf.set_kind(TCA_OPTIONS);
        let nla = DefaultNla {
            kind: buf.kind(),
            value: buf.value_mut().to_vec()
        };
        nlas.push(TcOpt::Other(nla));

        if let Some(correlations) = opts.correlations {
            // correlations/jitter
            let buf_size = 16;
            let mut raw_buf: Vec<u8> = vec![0; buf_size];
            NativeEndian::write_u16(&mut raw_buf[0..2], buf_size as u16);
            // delay jitter %
            NativeEndian::write_u32(&mut raw_buf[4..8], percent_to_buffer(correlations.delay_corr));
            // loss % jitter
            NativeEndian::write_u32(&mut raw_buf[8..12], percent_to_buffer(correlations.loss_corr));
            // duplicate % jitter
            NativeEndian::write_u32(&mut raw_buf[12..16], percent_to_buffer(correlations.dup_corr));

            let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
            buf.set_kind(TCA_NETEM_CORR);
            let nla = DefaultNla {
                kind: buf.kind(),
                value: buf.value_mut().to_vec()
            };
            nlas.push(TcOpt::Other(nla));
        }

        if let Some(reorder) = opts.reorder {
            let buf_size = 16;
            // reorderings
            let mut raw_buf: Vec<u8> = vec![0; buf_size];
            // bufsize
            NativeEndian::write_u16(&mut raw_buf[0..2], buf_size as u16);
            // reordered (as %)
            NativeEndian::write_u32(&mut raw_buf[4..8], percent_to_buffer(reorder.prob));
            // reordered jitter (as %)
            NativeEndian::write_u32(&mut raw_buf[8..12], percent_to_buffer(reorder.corr));

            let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
            buf.set_kind(TCA_NETEM_REORDER);
            let nla = DefaultNla {
                kind: buf.kind(),
                value: buf.value_mut().to_vec()
            };
            nlas.push(TcOpt::Other(nla));

        }

        // corruption
        if let Some(corrupt) = opts.corruption {
            let buf_size = 16;
            let mut raw_buf: Vec<u8> = vec![0; buf_size];
            NativeEndian::write_u16(&mut raw_buf[0..2], buf_size as u16);
            //  corrupted (as %)
            NativeEndian::write_u32(&mut raw_buf[4..8], percent_to_buffer(corrupt.prob));
            //  corrupted jitter (as %)
            NativeEndian::write_u32(&mut raw_buf[8..12], percent_to_buffer(corrupt.corr));
            let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
            buf.set_kind(TCA_NETEM_CORRUPT);
            let nla = DefaultNla {
                kind: buf.kind(),
                value: buf.value_mut().to_vec()
            };
            nlas.push(TcOpt::Other(nla));
        }

        if let Some(delay) = opts.delay {
            let buf_size = 16;
            // set delay
            let mut raw_buf: Vec<u8> = vec![0; buf_size];
            NativeEndian::write_u16(&mut raw_buf[0..2], buf_size as u16);
            NativeEndian::write_u64(&mut raw_buf[4..12], delay.stddev);
            let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
            buf.set_kind(10);
            let nla = DefaultNla {
                kind: buf.kind(),
                value: buf.value_mut().to_vec()
            };
            nlas.push(TcOpt::Other(nla));
            // set delay stddev
            let mut raw_buf: Vec<u8> = vec![0; buf_size];
            NativeEndian::write_u16(&mut raw_buf[0..2], buf_size as u16);
            NativeEndian::write_u64(&mut raw_buf[4..12], delay.delay);

            let mut buf = NlaBuffer::new_checked(raw_buf.as_mut_slice())?;
            buf.set_kind(11);
            let nla = DefaultNla {
                kind: buf.kind(),
                value: buf.value_mut().to_vec()
            };
            nlas.push(TcOpt::Other(nla));
        }


        self.message
            .nlas
            .push(nlas::Nla::Options(nlas));

        Ok(self)
    }
}

#[cfg(test)]
mod test {
    use std::{fs::File, os::unix::io::AsRawFd, path::Path, time::Duration};

    use futures::stream::TryStreamExt;
    use nix::sched::{setns, CloneFlags};
    use tokio::runtime::Runtime;

    use super::*;
    use crate::{
        new_connection,
        packet::{
            rtnl::tc::nlas::Nla::{HwOffload, Kind},
            LinkMessage,
            AF_UNSPEC,
        },
        NetworkNamespace,
        NETNS_PATH,
        SELF_NS_PATH,
    };

    const TEST_NS: &str = "netlink_test_qdisc_ns";
    const TEST_DUMMY: &str = "test_dummy";

    struct Netns {
        path: String,
        _cur: File,
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
            // setns(file.as_raw_fd(), CloneFlags::CLONE_NEWNET).unwrap();

            Self {
                path: path.to_string(),
                _cur: file,
                last,
            }
        }
    }
    impl Drop for Netns {
        fn drop(&mut self) {
            setns(self.last.as_raw_fd(), CloneFlags::CLONE_NEWNET).unwrap();

            let ns_path = Path::new(NETNS_PATH).join(&self.path);
            nix::mount::umount2(&ns_path, nix::mount::MntFlags::MNT_DETACH).unwrap();
            nix::unistd::unlink(&ns_path).unwrap();
            // _cur File will be closed auto
            // Since there is no async drop, NetworkNamespace::del cannot be called
            // here. Dummy interface will be deleted automatically after netns is
            // deleted.
        }
    }

    async fn setup_env() -> (Handle, LinkMessage, Netns) {
        let netns = Netns::new(TEST_NS).await;

        // Notice: The Handle can only be created after the setns, so that the
        // Handle is the connection within the new ns.
        let (connection, handle, _) = new_connection().unwrap();
        tokio::spawn(connection);
        handle
            .link()
            .add()
            .dummy(TEST_DUMMY.to_string())
            .execute()
            .await
            .unwrap();
        let mut links = handle
            .link()
            .get()
            .match_name(TEST_DUMMY.to_string())
            .execute();
        let link = links.try_next().await.unwrap();
        (handle, link.unwrap(), netns)
    }

    async fn test_async_new_qdisc_netem() {
        let (handle, link, netem) = setup_env().await;

        println!("idx: {:?}", link.header.index);

        let opts = NetemQdisc::default();


        handle
            .qdisc()
            .add(link.header.index as i32)
            .netem(opts)
            .unwrap()
            .execute()
            .await
            .unwrap();

        let mut qdiscs_iter = handle
            .qdisc()
            .get()
            .execute();

        println!("link header indx: {:?}", link.header.index);

        let mut found = false;

        while let Some(nl_msg) = qdiscs_iter.try_next().await.unwrap() {
            println!("nl_msg {:?}", nl_msg);
            if nl_msg.header.index == link.header.index as i32 {
                found = true;
                break
            }
        }

        if !found {
            panic!("netem qdisc not added");
        }

        handle
            .qdisc()
            .del(link.header.index as i32)
            .execute()
            .await
            .unwrap();

        let mut qdiscs_iter = handle
            .qdisc()
            .get()
            .execute();

        let mut found_deleted_qdisc = false;

        while let Some(nl_msg) = qdiscs_iter.try_next().await.unwrap() {
            if nl_msg.header.index == link.header.index as i32 { 
                found_deleted_qdisc = true;
                break
            }
        }
        if found_deleted_qdisc {
            panic!("qdisc did not get deleted");
        }
        drop(netem);
        drop(link);

    }

    async fn test_async_new_qdisc_ingress() {
        let (handle, test_link, _netns) = setup_env().await;
        handle
            .qdisc()
            .add(test_link.header.index as i32)
            .ingress()
            .execute()
            .await
            .unwrap();
        let mut qdiscs_iter = handle
            .qdisc()
            .get()
            .index(test_link.header.index as i32)
            .ingress()
            .execute();

        let mut found = false;
        while let Some(nl_msg) = qdiscs_iter.try_next().await.unwrap() {
            if nl_msg.header.index == test_link.header.index as i32
                && nl_msg.header.handle == 0xffff0000
            {
                assert_eq!(nl_msg.header.family, AF_UNSPEC as u8);
                assert_eq!(nl_msg.header.handle, 0xffff0000);
                assert_eq!(nl_msg.header.parent, TC_H_INGRESS);
                assert_eq!(nl_msg.header.info, 1); // refcount
                assert_eq!(nl_msg.nlas[0], Kind("ingress".to_string()));
                assert_eq!(nl_msg.nlas[2], HwOffload(0));
                found = true;
                break;
            }
        }
        if !found {
            panic!("not found dev:{} qdisc.", test_link.header.index);
        }
    }

    #[test]
    fn test_new_qdisc_ingress() {
        Runtime::new().unwrap().block_on(test_async_new_qdisc_ingress());
    }

    #[test]
    fn test_new_qdisc_netem() {
        Runtime::new().unwrap().block_on(test_async_new_qdisc_netem());
    }
}
