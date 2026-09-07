use asdf_overlay_common::ipc::ServerToClientPacket;
use asdf_overlay_event::{AugmentCard, AugmentChoices, OverlayEvent};

#[test]
fn augment_choices_round_trip_through_ipc_packet() {
    let expected = AugmentChoices {
        mode: "mayhem".into(),
        cards: vec![AugmentCard {
            instance: 7,
            name: "Test Augment".into(),
            description: "Test Description".into(),
            x: 120.0,
            y: 240.0,
            width: 300.0,
            height: 420.0,
        }],
    };
    let packet = ServerToClientPacket::Event(OverlayEvent::LolAugmentChoices(expected.clone()));

    let encoded = bincode::encode_to_vec(packet, bincode::config::standard()).unwrap();
    let (decoded, consumed) = bincode::decode_from_slice::<ServerToClientPacket, _>(
        &encoded,
        bincode::config::standard(),
    )
    .unwrap();

    assert_eq!(consumed, encoded.len());
    let ServerToClientPacket::Event(OverlayEvent::LolAugmentChoices(actual)) = decoded else {
        panic!("decoded packet was not an augment choices event");
    };
    assert_eq!(actual, expected);
}
