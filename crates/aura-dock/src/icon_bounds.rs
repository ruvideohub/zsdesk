//! Detect severely undersized images in a large transparent Shell icon canvas.
/// Returns a square crop only for content occupying at most half the canvas.
/// Ordinary icons retain their artist-supplied transparent padding.
pub fn tiny_content_crop(width:u32,height:u32,pixels:&[u8])->Option<(u32,u32,u32,u32)> {
    let n=(width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if width<8 || height<8 || width>2048 || height>2048 || pixels.len()!=n {return None;}
    let(mut left,mut top,mut right,mut bottom)=(width,height,0,0);let mut count=0u32;
    for y in 0..height {for x in 0..width {
        if pixels[((y*width+x)*4+3) as usize]>=16 {left=left.min(x);top=top.min(y);right=right.max(x);bottom=bottom.max(y);count+=1;}
    }}
    if count<9 {return None;}
    let w=right-left+1;let h=bottom-top+1;
    if w*2>width || h*2>height {return None;}
    let side=(w.max(h)+2).min(width.min(height));
    let cx=(left+right)/2;let cy=(top+bottom)/2;
    let x=cx.saturating_sub(side/2).min(width-side);let y=cy.saturating_sub(side/2).min(height-side);
    Some((x,y,side,side))
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn tiny_square_is_enlarged_without_stretching(){
        let mut px=vec![0;256*256*4];for y in 112..144 {for x in 112..144{px[(y*256+x)*4+3]=255;}}
        assert_eq!(tiny_content_crop(256,256,&px),Some((110,110,34,34)));
    }
    #[test] fn ordinary_or_invisible_icons_are_not_cropped(){
        assert_eq!(tiny_content_crop(32,32,&vec![0;32*32*4]),None);
        assert_eq!(tiny_content_crop(32,32,&vec![255;32*32*4]),None);
        assert_eq!(tiny_content_crop(32,32,&[0;4]),None);
    }
}
