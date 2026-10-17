use lucas_overlay::{Event, Overlay, OverlayError, Pulse, Recording, Rect};

#[test]
fn commands_are_recorded_in_order_and_shutdown_is_idempotent() {
    let original = Rect::new(-300.0, 400.0, -100.0, 600.0).unwrap();
    let moved = Rect::new(20.5, 50.5, 350.5, 380.5).unwrap();
    let mut overlay = Recording::new(original).unwrap();
    let pulse = Pulse::default();
    overlay.show_pulse(pulse).unwrap();
    overlay.set_target_rect(moved).unwrap();
    overlay.reassert_topmost().unwrap();
    overlay.hide().unwrap();
    overlay.shutdown().unwrap();
    overlay.shutdown().unwrap();
    assert_eq!(overlay.target_rect(), moved);
    assert_eq!(
        overlay.events(),
        &[
            Event::Show(pulse),
            Event::Target(moved),
            Event::ReassertTopmost,
            Event::Hide,
            Event::Shutdown,
        ]
    );
    let closed: &mut dyn Overlay = &mut overlay;
    assert_eq!(closed.show_pulse(pulse), Err(OverlayError::Closed));
    assert_eq!(closed.hide(), Err(OverlayError::Closed));
    assert_eq!(closed.set_target_rect(original), Err(OverlayError::Closed));
    assert_eq!(closed.reassert_topmost(), Err(OverlayError::Closed));
}

#[test]
fn pulse_accepts_core_salience_and_defaults_to_medium() {
    assert_eq!(Pulse::default().salience, lucas_core::Salience::Medium);
    let pulse = Pulse {
        salience: lucas_core::Salience::Soft,
        ..Pulse::default()
    };
    assert_eq!(pulse.ripple_count(), 1);
}

#[test]
fn invalid_update_preserves_state_and_log() {
    let original = Rect::new(10.0, 20.0, 210.0, 220.0).unwrap();
    let mut overlay = Recording::new(original).unwrap();
    let huge = Rect::new(0.0, 0.0, 1e30, 1e30).unwrap();
    assert_eq!(
        overlay.set_target_rect(huge),
        Err(OverlayError::InvalidRect)
    );
    assert_eq!(overlay.target_rect(), original);
    assert!(overlay.events().is_empty());
    assert!(Recording::new(huge).is_err());
}
