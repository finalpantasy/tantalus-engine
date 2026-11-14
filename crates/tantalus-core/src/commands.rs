use tantalus_contracts::{EffectTimeline, EngineError, ErrorCategory};

pub fn validate_timeline(timeline: &EffectTimeline) -> Result<(), EngineError> {
    if timeline.duration_ticks == 0 {
        return Err(invalid_timeline("effect duration must be nonzero"));
    }

    let mut previous_offset = 0;
    for marker in &timeline.markers {
        if marker.tick_offset == 0 || marker.tick_offset > timeline.duration_ticks {
            return Err(invalid_timeline(format!(
                "marker `{}` must occur after effect start and no later than completion",
                marker.event
            )));
        }
        if marker.tick_offset < previous_offset {
            return Err(invalid_timeline(
                "timeline markers must be ordered by nondecreasing tick offset",
            ));
        }
        previous_offset = marker.tick_offset;
    }
    Ok(())
}

fn invalid_timeline(context: impl Into<String>) -> EngineError {
    EngineError::new(
        ErrorCategory::InvalidProperties,
        context,
        "tantalus-core/commands",
        true,
    )
}
