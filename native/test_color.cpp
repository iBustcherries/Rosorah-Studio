#include "color.hpp"
#include <cassert>
#include <cstdio>
using namespace rosorah;
int main() {
    ColorConverter converter;std::vector<uint8_t> out;
    uint32_t red=1023u|(3u<<30);
    converter.convert(&red,1,{Pixel::Rgb10,Transfer::Srgb},out);assert(out==std::vector<uint8_t>({255,0,0,255}));
    converter.convert(&red,1,{Pixel::Bgr10,Transfer::Srgb},out);assert(out==std::vector<uint8_t>({0,0,255,255}));
    uint16_t half[]={0x3c00,0x3c00,0x3c00,0x3c00}; // 80 nits -> sRGB approx 168/255 with 203-nit white.
    converter.convert(half,1,{Pixel::Rgba16f,Transfer::Linear709},out);assert(out[0]>=168 && out[0]<=169 && out[1]==out[0] && out[2]==out[0]);
    assert(half_float(0x0000)==0 && half_float(0x8000)==0 && half_float(0x3c00)==1 && half_float(0xc000)==-2);
    assert(half_float(0x0001)>0 && half_float(0x7e00)==0);
    for(auto transfer:{Transfer::Pq2020,Transfer::Hlg2020}) {
        unsigned previous=0;
        for(unsigned code=0;code<1024;code++) {
            uint32_t pixel=code|(code<<10)|(code<<20)|(3u<<30);
            converter.convert(&pixel,1,{Pixel::Rgb10,transfer},out);
            assert(out[0]>=previous && out[1]==out[0] && out[2]==out[0] && out[3]==255);previous=out[0];
        }
        assert(previous>=(transfer==Transfer::Hlg2020?253u:254u));
    }
    // Independent ST2084 reference: code 0.508078 maps to 100 nits.
    unsigned code=520;uint32_t gray=code|(code<<10)|(code<<20);
    converter.convert(&gray,1,{Pixel::Rgb10,Transfer::Pq2020},out);assert(out[0]>=185 && out[0]<=187);
    puts("PASS: 10-bit packing, scRGB half floats, PQ reference luminance, neutral/monotonic PQ and HLG SDR output");
}
