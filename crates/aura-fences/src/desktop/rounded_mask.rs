//! Antialiased mask for premultiplied BGRA layered-window pixels.
//!
//! D2D's rectangular content clip can otherwise paint outside the panel's
//! rounded body, particularly at the bottom corners after scrolling/selection.
pub fn apply_bgra(pixels: &mut [u8], width: usize, height: usize, radius: f32) {
    let Some(required) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else { return };
    if width == 0 || height == 0 || pixels.len() != required || !radius.is_finite() || radius <= 0.0 { return; }
    let r = radius.min(width.min(height) as f32 / 2.0);
    let extent = (r.ceil() as usize).min(width.min(height) / 2);
    if extent == 0 { return; }
    // Evaluate only the four corners; the centre is unchanged. 4×4 coverage
    // retains smooth edges and scales colour and alpha together (PBGRA).
    for y in 0..extent {
        for x in 0..extent {
            let mut covered = 0u16;
            for sy in 0..4 { for sx in 0..4 {
                let dx = r - (x as f32 + (sx as f32 + 0.5) / 4.0);
                let dy = r - (y as f32 + (sy as f32 + 0.5) / 4.0);
                if dx <= 0.0 || dy <= 0.0 || dx*dx + dy*dy <= r*r { covered += 1; }
            }}
            if covered == 16 { continue; }
            for (px,py) in [(x,y),(width-1-x,y),(x,height-1-y),(width-1-x,height-1-y)] {
                let start=(py*width+px)*4;
                for channel in &mut pixels[start..start+4] {
                    *channel = ((u16::from(*channel)*covered+8)/16) as u8;
                }
            }
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn corners_are_transparent_and_center_is_untouched() {
        let mut pixels=vec![200u8;40*30*4];
        apply_bgra(&mut pixels,40,30,10.0);
        for &(x,y) in &[(0,0),(39,0),(0,29),(39,29)] { assert_eq!(&pixels[(y*40+x)*4..(y*40+x)*4+4],&[0;4]); }
        assert_eq!(&pixels[(15*40+20)*4..(15*40+20)*4+4],&[200;4]);
        assert!(pixels.chunks_exact(4).any(|p|p[3]>0 && p[3]<200));
        assert!(pixels.chunks_exact(4).all(|p|p.iter().all(|v|*v==p[3])));
    }
    #[test] fn zero_radius_invalid_buffers_and_tiny_sizes_are_safe() {
        let mut a=vec![128;8*8*4];let original=a.clone();apply_bgra(&mut a,8,8,0.0);assert_eq!(a,original);
        apply_bgra(&mut a,9,8,5.0);assert_eq!(a,original);
        let mut one=vec![255;4];apply_bgra(&mut one,1,1,99.0);assert_eq!(one,vec![255;4]);
    }
}
