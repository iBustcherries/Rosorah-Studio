// Independent SDR conversion for capture. HDR remains enabled in the game.
#pragma once
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>
namespace rosorah {
enum class Pixel { Rgba8, Bgra8, Rgb10, Bgr10, Rgba16f };
enum class Transfer { Srgb, Pq2020, Hlg2020, Linear709 };
struct Color {
    Pixel pixel=Pixel::Rgba8;
    Transfer transfer=Transfer::Srgb;
    unsigned bytes() const {return pixel==Pixel::Rgba16f?8:4;}
    bool hdr() const {return transfer!=Transfer::Srgb;}
    bool convert() const {return hdr() || (pixel!=Pixel::Rgba8 && pixel!=Pixel::Bgra8);}
};
inline float half_float(uint16_t v) {
    const uint32_t sign=uint32_t(v&0x8000)<<16, exp=(v>>10)&31, mant=v&1023;
    if(exp==31)return mant?0.f:((sign)?0.f:10000.f);
    if(!exp)return (sign?-1.f:1.f)*std::ldexp(float(mant),-24);
    uint32_t word=sign|((exp+112)<<23)|(mant<<13);float f;memcpy(&f,&word,4);return f;
}
class ColorConverter {
    std::array<float,1024> pq{},hlg{};
    std::array<uint8_t,4097> gamma{};
    static float positive(float x){return std::isfinite(x)?std::max(0.f,x):0.f;}
    uint8_t encode(float x) const {return gamma[unsigned(std::clamp(x,0.f,1.f)*4096.f+.5f)];}
public:
    ColorConverter() {
        constexpr double m1=2610./16384.,m2=2523./32.,c1=3424./4096.,c2=2413./128.,c3=2392./128.;
        for(unsigned i=0;i<1024;i++) {
            double code=double(i)/1023.,p=std::pow(code,1./m2);
            pq[i]=float(10000.*std::pow(std::max(p-c1,0.)/(c2-c3*p),1./m1));
            // BT.2100 HLG, reference 1000-nit display / system gamma 1.2.
            double scene=code<=.5 ? code*code/3. : (std::exp((code-.55991073)/.17883277)+.28466892)/12.;
            hlg[i]=float(scene);
        }
        for(unsigned i=0;i<gamma.size();i++) {
            double v=double(i)/4096.;double g=v<=.0031308?12.92*v:1.055*std::pow(v,1./2.4)-.055;
            gamma[i]=uint8_t(std::clamp(g*255.+.5,0.,255.));
        }
    }
    void convert(const void* input,size_t count,Color color,std::vector<uint8_t>& output) const {
        output.resize(count*4);
        const auto* source=static_cast<const uint8_t*>(input);
        for(size_t i=0;i<count;i++,source+=color.bytes()) {
            auto* target=&output[i*4];float r=0,g=0,b=0;
            if(color.pixel==Pixel::Rgba16f) {
                uint16_t channels[4];memcpy(channels,source,8);
                // scRGB 1.0 is 80 nits (linear BT.709 primaries).
                r=half_float(channels[0])*80.f;g=half_float(channels[1])*80.f;b=half_float(channels[2])*80.f;
            } else if(color.pixel==Pixel::Rgb10 || color.pixel==Pixel::Bgr10) {
                uint32_t packed;memcpy(&packed,source,4);
                unsigned red=packed&1023,green=(packed>>10)&1023,blue=(packed>>20)&1023;
                if(color.pixel==Pixel::Bgr10)std::swap(red,blue);
                if(color.transfer==Transfer::Srgb){target[0]=uint8_t((red*255+511)/1023);target[1]=uint8_t((green*255+511)/1023);target[2]=uint8_t((blue*255+511)/1023);target[3]=255;continue;}
                const auto& lut=color.transfer==Transfer::Hlg2020?hlg:pq;
                r=lut[red];g=lut[green];b=lut[blue];
            } else {
                target[0]=source[color.pixel==Pixel::Bgra8?2:0];target[1]=source[1];target[2]=source[color.pixel==Pixel::Bgra8?0:2];target[3]=255;continue;
            }
            if(color.transfer==Transfer::Hlg2020) {
                const float luminance=.2627f*r+.6780f*g+.0593f*b;
                const float gain=1000.f*std::pow(std::max(luminance,0.f),.2f);
                r*=gain;g*=gain;b*=gain;
            }
            if(color.transfer==Transfer::Pq2020 || color.transfer==Transfer::Hlg2020) {
                // Linear BT.2020 -> linear BT.709/D65; clamp out-of-gamut light.
                const float rr=1.660491f*r-.587641f*g-.072850f*b;
                const float gg=-.124550f*r+1.132900f*g-.008349f*b;
                const float bb=-.018151f*r-.100579f*g+1.118730f*b;
                r=rr;g=gg;b=bb;
            }
            r=positive(r)/203.f;g=positive(g)/203.f;b=positive(b)/203.f;
            const float luminance=.2126f*r+.7152f*g+.0722f*b;
            // Preserve diffuse/midtone levels, then smoothly compress highlights.
            // Fixed 203-nit reference white; SDR canvas, not HDR passthrough.
            if(luminance>.75f) {
                const float above=luminance-.75f;
                const float gain=(.75f+.25f*above/(above+.25f))/luminance;
                r*=gain;g*=gain;b*=gain;
            }
            target[0]=encode(r);target[1]=encode(g);target[2]=encode(b);target[3]=255;
        }
    }
};
}
