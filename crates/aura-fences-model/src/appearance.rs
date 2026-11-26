//! Portable, testable controls for the Fences appearance panel.
use crate::desktop::DesktopSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control { IconScale, Spacing, Padding, Opacity, Radius, Edge, Blur }
impl Control {
    pub const ALL: [Self;7] = [Self::IconScale,Self::Spacing,Self::Padding,Self::Opacity,Self::Radius,Self::Edge,Self::Blur];
    pub fn label(self)->&'static str { match self {
        Self::IconScale=>"图标缩放", Self::Spacing=>"图标间距",Self::Padding=>"内部边距",
        Self::Opacity=>"整体不透明度",Self::Radius=>"圆角大小",Self::Edge=>"边框高光",Self::Blur=>"背景模糊",
    }}
    pub fn range(self)->(i32,i32) {match self {Self::IconScale=>(50,200),Self::Spacing=>(0,32),Self::Padding=>(0,24),Self::Opacity=>(10,100),Self::Radius=>(0,40),Self::Edge=>(0,100),Self::Blur=>(0,1)}}
    pub fn unit(self)->&'static str {match self {Self::IconScale|Self::Opacity|Self::Edge=>"%",Self::Blur=>"",_=>" DIP"}}
    pub fn get(self,s:&DesktopSettings)->i32 {match self {Self::IconScale=>i32::from(s.icon_scale),Self::Spacing=>i32::from(s.icon_spacing),Self::Padding=>i32::from(s.inner_padding),Self::Opacity=>i32::from(s.opacity),Self::Radius=>i32::from(s.corner_radius),Self::Edge=>i32::from(s.edge_highlight),Self::Blur=>i32::from(s.blur)}}
    pub fn set(self,s:&mut DesktopSettings,value:i32) {
        let (min,max)=self.range();let n=value.clamp(min,max);
        match self {Self::IconScale=>s.icon_scale=n as u16,Self::Spacing=>s.icon_spacing=n as u8,Self::Padding=>s.inner_padding=n as u8,
            Self::Opacity=>s.opacity=n as u8,Self::Radius=>s.corner_radius=n as u8,Self::Edge=>s.edge_highlight=n as u8,Self::Blur=>s.blur=n!=0}
    }
    pub fn from_fraction(self, fraction:f32)->i32 {
        let (min,max)=self.range();if !fraction.is_finite(){return min;}
        (min as f32 + (max-min) as f32*fraction.clamp(0.0,1.0)).round() as i32
    }
    pub fn fraction(self,s:&DesktopSettings)->f32 {let (min,max)=self.range();(self.get(s).clamp(min,max)-min) as f32/(max-min) as f32}
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn controls_clamp_and_are_independent(){let mut s=DesktopSettings::default();let original=s.clone();
        for c in Control::ALL {let mut copy=s.clone();c.set(&mut copy,-1000);assert_eq!(c.get(&copy),c.range().0);c.set(&mut copy,9999);assert_eq!(c.get(&copy),c.range().1);}
        Control::IconScale.set(&mut s,150);assert_eq!(s.icon_scale,150);assert_eq!(s.opacity,original.opacity);assert_eq!(s.icon_spacing,original.icon_spacing);
    }
    #[test]fn slider_positions_span_their_range(){let mut s=DesktopSettings::default();for c in Control::ALL {
        assert_eq!(c.from_fraction(0.0),c.range().0);assert_eq!(c.from_fraction(1.0),c.range().1);
        c.set(&mut s,c.range().0);assert_eq!(c.fraction(&s),0.0);c.set(&mut s,c.range().1);assert_eq!(c.fraction(&s),1.0);
    }}
}
