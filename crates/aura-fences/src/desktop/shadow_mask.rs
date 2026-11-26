//! Soft, premultiplied BGRA outline for a separate click-through shadow window.
//! The fence interior remains transparent, so its translucent fill is not
//! darkened by its own shadow.
fn distance(x:f32,y:f32,w:f32,h:f32,r:f32)->f32 {
    let r=r.min(w.min(h)/2.0).max(0.0);
    let qx=(x-w/2.0).abs()-(w/2.0-r);
    let qy=(y-h/2.0).abs()-(h/2.0-r);
    (qx.max(0.0).powi(2)+qy.max(0.0).powi(2)).sqrt()+qx.max(qy).min(0.0)-r
}
/// `w`, `h`, `margin` and `offset` are physical pixels. Output must already
/// be zero-filled. Reject oversized/malformed requests without modifying data.
pub fn paint(pixels:&mut [u8],w:usize,h:usize,margin:usize,radius:f32,offset:f32,strength:u8)->bool {
    let Some(tw)=w.checked_add(margin.saturating_mul(2)) else{return false};
    let Some(th)=h.checked_add(margin.saturating_mul(2)) else{return false};
    let Some(len)=tw.checked_mul(th).and_then(|n|n.checked_mul(4)) else{return false};
    if w==0||h==0||margin==0||len!=pixels.len()||!radius.is_finite()||!offset.is_finite(){return false;}
    pixels.fill(0);
    if strength==0{return true;}
    let (wf,hf,mf)=(w as f32,h as f32,margin as f32);
    let sigma=(mf*0.46).max(1.0);
    let max_alpha=f32::from(strength.min(100))*1.05;
    // The central bands are inside the body, so only the perimeter needs
    // evaluation; the large transparent middle remains zero-filled.
    let side=(margin+(radius.ceil().max(0.0) as usize).min(w.min(h)/2)+2).min(tw/2);
    let rounded=(radius.ceil().max(0.0) as usize).min(w.min(h)/2)+2;
    for y in 0..th {
        let middle=y>=margin.saturating_add(rounded) && y<margin.saturating_add(h).saturating_sub(rounded);
        let ranges: [(usize,usize);2]=if middle {[(0,side),(tw-side,tw)]} else {[(0,tw),(0,0)]};
        for (start,end) in ranges {for x in start..end {
            let lx=x as f32+0.5-mf;let ly=y as f32+0.5-mf;
            let outside=distance(lx,ly,wf,hf,radius);
            if outside<=0.0 {continue;}
            let shifted=distance(lx,ly-offset,wf,hf,radius).max(0.0);
            let coverage=outside.clamp(0.0,1.0);
            let alpha=(max_alpha*coverage*(-shifted*shifted/(2.0*sigma*sigma)).exp()).round().clamp(0.0,255.0) as u8;
            pixels[(y*tw+x)*4+3]=alpha;
        }}
    }
    true
}
#[cfg(test)] mod tests {
    use super::*;
    fn render(w:usize,h:usize,m:usize,r:f32,offset:f32,a:u8)->Vec<u8>{let mut p=vec![255;(w+2*m)*(h+2*m)*4];assert!(paint(&mut p,w,h,m,r,offset,a));p}
    #[test]fn body_is_clear_and_shadow_is_premultiplied(){
        let (w,h,m)=(80,50,12);let p=render(w,h,m,10.0,3.0,70);let tw=w+2*m;
        for y in m+10..m+h-10 {for x in m+10..m+w-10{assert_eq!(&p[(y*tw+x)*4..(y*tw+x)*4+4],&[0,0,0,0]);}}
        assert!(p.chunks_exact(4).any(|v|v[3]>0));
        assert!(p.chunks_exact(4).all(|v|v[0]==0&&v[1]==0&&v[2]==0&&v[3]<=105));
    }
    #[test]fn strength_zero_and_invalid_input_are_safe(){
        let p=render(10,10,4,0.0,1.0,0);assert!(p.iter().all(|v|*v==0));
        let mut p=vec![17;8];assert!(!paint(&mut p,2,2,4,10.0,0.0,80));assert_eq!(p,vec![17;8]);
        assert!(!paint(&mut p,usize::MAX,1,4,10.0,0.0,80));
    }
    #[test]fn shadow_fades_and_offset_favors_bottom(){
        let(w,h,m)=(80,50,12);let p=render(w,h,m,8.0,3.0,100);let tw=w+2*m;let a=|x,y|p[(y*tw+x)*4+3];
        assert!(a(m+w/2,m-2)<a(m+w/2,m+h+1));
        assert!(a(m+w/2,m+h+1)>a(m+w/2,m+h+10));
    }
}
