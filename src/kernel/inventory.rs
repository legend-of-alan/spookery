use crate::error::Reason;

pub fn clamp_need(current: i64, delta: i64, min: Option<i64>, max: Option<i64>) -> Result<i64, Reason> {
    let mut next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if let Some(min) = min {
        if next < min {
            next = min;
        }
    }
    if let Some(max) = max {
        if next > max {
            next = max;
        }
    }
    Ok(next)
}

pub fn stock_next(current: i64, delta: i64) -> Result<i64, Reason> {
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 {
        return Err(Reason::OutOfRange);
    }
    Ok(next)
}

pub fn resource_next(current: i64, delta: i64) -> Result<i64, Reason> {
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 {
        return Err(Reason::OutOfRange);
    }
    Ok(next)
}
