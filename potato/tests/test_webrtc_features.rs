#![cfg(feature = "std")]
//! WebRTC Features编译测试
//!
//! 验证webrtc feature可以正确编译

#[test]
fn test_webrtc_feature_compilation() {
    // 此测试验证webrtc feature被正确启用
    // 实际的功能测试需要完整的WebRTC环境

    #[cfg(feature = "webrtc")]
    {
        // 验证webrtc模块存在
        // 这些类型在webrtc feature启用时应该可用
        use potato::webrtc::WebRTCConfig;
        use potato::webrtc::WebRTCSFU;

        // 验证配置可以创建
        let config = WebRTCConfig::default();
        assert_eq!(config.max_peers, 100);
        // 验证SFU可以创建
        let _sfu = WebRTCSFU::new(config);
    }

    #[cfg(not(feature = "webrtc"))]
    {
        // 如果没有启用webrtc feature,测试应该跳过
        println!("WebRTC feature not enabled, skipping test");
    }
}

#[test]
fn test_webrtc_config_defaults() {
    #[cfg(feature = "webrtc")]
    {
        use potato::webrtc::WebRTCConfig;

        let config = WebRTCConfig::default();

        // 验证默认配置
        assert_eq!(config.max_peers, 100);
        assert!(config
            .ice_servers
            .contains(&"stun:stun.l.google.com:19302".to_string()));
        assert_eq!(config.ws_path, "/ws");
    }
}

#[test]
fn webrtc_builder_installs_websocket_signaling_route() {
    #[cfg(feature = "webrtc")]
    {
        let mut server = potato::HttpServer::new("127.0.0.1:0");
        server
            .configure(|ctx| {
                ctx.use_webrtc().ws_path("/rtc").finish();
            })
            .unwrap();
    }
}

#[cfg(feature = "webrtc")]
mod subscribe_lifecycle {
    use rtc::media_stream::MediaStreamTrack;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use std::sync::Arc;
    use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
    use webrtc::peer_connection::{
        MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    };

    struct NoopHandler;

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for NoopHandler {}

    async fn peer_connection() -> anyhow::Result<Arc<dyn PeerConnection>> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        Ok(Arc::new(
            PeerConnectionBuilder::new()
                .with_media_engine(media_engine)
                .with_handler(Arc::new(NoopHandler))
                .with_udp_addrs(vec!["127.0.0.1:0".to_string()])
                .build()
                .await?,
        ))
    }

    #[tokio::test]
    async fn subscribe_negotiates_after_publisher_is_registered() {
        // The managed sandbox used by local validation can deny UDP sockets; the
        // full negotiation still runs in normal development and CI environments.
        if std::net::UdpSocket::bind("127.0.0.1:0").is_err() {
            eprintln!("skipping WebRTC negotiation: UDP sockets are unavailable");
            return;
        }

        let sfu = Arc::new(potato::webrtc::WebRTCSFU::new(
            potato::webrtc::WebRTCConfig::default(),
        ));
        sfu.create_room("room", None).await.unwrap();

        let publisher = peer_connection().await.unwrap();
        publisher
            .add_track(Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
                "publisher-stream".to_string(),
                "publisher-track".to_string(),
                "publisher-video".to_string(),
                RtpCodecKind::Video,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(0x1234_5678),
                        ..Default::default()
                    },
                    codec: RTCRtpCodec {
                        mime_type: "video/VP8".to_string(),
                        clock_rate: 90_000,
                        channels: 0,
                        sdp_fmtp_line: String::new(),
                        rtcp_feedback: vec![],
                    },
                    ..Default::default()
                }],
            )))
                as Arc<dyn webrtc::media_stream::track_local::TrackLocal>)
            .await
            .unwrap();
        let publisher_offer = publisher.create_offer(None).await.unwrap();
        publisher
            .set_local_description(publisher_offer)
            .await
            .unwrap();
        let publisher_offer = publisher.local_description().await.unwrap();
        sfu.handle_offer("publisher", "room", &publisher_offer.sdp)
            .await
            .unwrap();

        let subscriber = peer_connection().await.unwrap();
        subscriber
            .add_transceiver_from_kind(RtpCodecKind::Video, None)
            .await
            .unwrap();
        let subscriber_offer = subscriber.create_offer(None).await.unwrap();
        subscriber
            .set_local_description(subscriber_offer)
            .await
            .unwrap();
        let subscriber_offer = subscriber.local_description().await.unwrap();
        let subscriber_answer = sfu
            .handle_subscribe("subscriber", "room", "publisher", &subscriber_offer.sdp)
            .await
            .unwrap();

        assert!(subscriber_answer.contains("m=video"));
        let room = sfu.get_room("room").await.unwrap();
        let subscriber_peer = room.get_peer("subscriber").await.unwrap();
        assert_eq!(subscriber_peer.subscribing, vec!["publisher"]);
        subscriber
            .set_remote_description(
                webrtc::peer_connection::RTCSessionDescription::answer(subscriber_answer)
                    .expect("valid subscriber answer"),
            )
            .await
            .unwrap();
    }
}
