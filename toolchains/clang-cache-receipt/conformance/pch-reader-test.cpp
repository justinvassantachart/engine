#include "WebIDECacheReceiptPCHReader.h"
#include <cassert>
#include <cstdio>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>
using namespace clang::webide;
using namespace clang::webide::pch_detail;
struct Writer {
  std::vector<unsigned char> Data{'C','P','C','H'};
  uint64_t Bit = 32;
  void bits(uint64_t V, unsigned N) {
    for (unsigned I = 0; I < N; ++I, ++Bit) {
      if (Data.size() <= Bit/8) Data.push_back(0);
      Data[Bit/8] |= ((V >> I)&1) << (Bit%8);
    }
  }
  void vbr(uint64_t V, unsigned N) {
    uint64_t Mask=(uint64_t(1)<<(N-1))-1;
    do {uint64_t Word=V&Mask;V>>=N-1;bits(Word|(V?Mask+1:0),N);}while(V);
  }
  void align() { while (Bit%32) bits(0,1); }
  void blob(const std::string &S) {vbr(S.size(),6);align();for(unsigned char C:S)bits(C,8);align();}
};
struct Fixture {
 Writer W;
 uint64_t Record, Time, Flags, NameLength, Hash, Tail, End;
 size_t Offset;
 Fixture(std::string Name="/header.h", uint64_t Stamp=0, unsigned Flag=0,
         std::string Actual="", uint64_t Requested=UINT64_MAX) {
  Record=W.Bit;W.bits(4,4);W.vbr(1,6);W.vbr(5,12);Time=W.Bit;W.vbr(Stamp,32);
  Flags=W.Bit;W.bits(Flag,4);NameLength=W.Bit;W.vbr(Requested==UINT64_MAX?Name.size():Requested,16);W.blob(Name+Actual);
  Hash=W.Bit;W.bits(5,4);W.bits(0,32);W.bits(0,32);Tail=W.Bit;W.bits(0,4);W.align();End=W.Bit;
  Offset=W.Data.size();W.bits(0,64);
 }
 InputLayout layout(bool Stamps=false) {return{W.Data.data(),W.Data.size(),W.Data.data()+Offset,8,1,Record,End,Stamps};}
 void set(uint64_t Pos,unsigned Width,uint64_t Value) {for(unsigned I=0;I<Width;++I){auto &B=W.Data[(Pos+I)/8];unsigned Shift=(Pos+I)%8;B=(B&~(1u<<Shift))|(((Value>>I)&1)<<Shift);}}
};
struct Seen {size_t Count=0;bool Accept=true;bool Print=false;};
bool sink(void *P,const char *Name,size_t Size) noexcept {auto &S=*static_cast<Seen*>(P);++S.Count;if(S.Print)std::printf("%.*s\n",int(Size),Name);return S.Accept;}
PCHReceiptProof run(Fixture &F,bool Stamps=false){Seen S;return inspect(F.layout(Stamps),&S,sink);}
int main(int Argc,char **Argv) {
 if(Argc==8){
  std::ifstream In(Argv[1],std::ios::binary);std::vector<unsigned char> Bytes((std::istreambuf_iterator<char>(In)),{});
  uint64_t Base=std::stoull(Argv[2]),End=std::stoull(Argv[3]);size_t Offset=std::stoull(Argv[4]),Count=std::stoull(Argv[5]);bool Stamps=std::stoull(Argv[6]);
  assert(Offset<=Bytes.size() && Count*8<=Bytes.size()-Offset);Seen S;S.Print=std::string(Argv[7])=="paths";
  auto P=inspect({Bytes.data(),Bytes.size(),Bytes.data()+Offset,Count*8,Count,Base,End,Stamps},&S,sink);
  std::fprintf(stderr,"complete=%d independent=%d count=%zu reason=%s\n",P.complete,P.metadataIndependent,S.Count,P.unsupported?P.unsupported:"none");return P.complete?0:2;
 }
 Fixture Good;auto P=run(Good);assert(P.complete&&P.metadataIndependent);assert(run(Good,true).complete&&!run(Good,true).metadataIndependent);
 Fixture Time("/header.h",1);assert(run(Time).complete&&!run(Time).metadataIndependent);
 Fixture MaxTime("/header.h",UINT64_MAX);assert(run(MaxTime).complete&&!run(MaxTime).metadataIndependent);
 for(unsigned Flag:{1,2,8}){Fixture F("/header.h",0,Flag);assert(!run(F).complete);}Fixture Top("/header.h",0,4);assert(run(Top).complete);
 for(const auto *Name:{"", "/", "relative", "/a/../b", "/a/./b", "/a//b", "/a/", "/a\\b", "/a\nb", "/a\x7f"}){Fixture F(Name);assert(!run(F).complete);}
 Fixture Null(std::string("/x\0y",4));assert(!run(Null).complete);
 Fixture Long(std::string("/")+std::string(4096,'x'));assert(!run(Long).complete);
 Fixture MaxName(std::string("/")+std::string(4095,'x'));assert(run(MaxName).complete);
 Fixture Alias("/a",0,0,"/b");assert(!run(Alias).complete);Fixture Same("/a",0,0,"/a");assert(run(Same).complete);
 Fixture BadName("/a",0,0,"",16383);assert(!run(BadName).complete);
 Fixture BadHash;BadHash.set(BadHash.Hash,4,4);assert(!run(BadHash).complete);
 Fixture BadTail;BadTail.set(BadTail.Tail,4,1);assert(!run(BadTail).complete);
 Fixture BadID;BadID.set(BadID.Record+4,6,2);assert(!run(BadID).complete);
 Fixture Offset;Offset.W.Data[Offset.Offset]=1;assert(!run(Offset).complete);
 Seen Reject;Reject.Accept=false;assert(!inspect(Good.layout(),&Reject,sink).complete);
 for(size_t N=0;N<Good.W.Data.size();++N){auto L=Good.layout();L.Bytes=N;Seen S;assert(!inspect(L,&S,sink).complete);}
 for(size_t Count:{size_t(0),size_t(2),size_t(16385),SIZE_MAX}){auto L=Good.layout();L.Count=Count;Seen S;assert(!inspect(L,&S,sink).complete);}
 for(uint64_t Base:{uint64_t(0),Good.End,UINT64_MAX}){auto L=Good.layout();L.BaseBit=Base;Seen S;assert(!inspect(L,&S,sink).complete);}
 for(uint64_t End:{uint64_t(0),Good.End-1,UINT64_MAX}){auto L=Good.layout();L.EndBit=End;Seen S;assert(!inspect(L,&S,sink).complete);}
 auto L=Good.layout();L.Offsets=reinterpret_cast<const unsigned char*>(uintptr_t(1));Seen S;assert(!inspect(L,&S,sink).complete);
 // Every individual bit mutation and every offset-table byte value remains safe.
 // Some changes (for example hash values or top-level flags) are admissible;
 // object/owned-input identity still includes the entire PCH's digest.
 for(size_t Byte=0;Byte<Good.W.Data.size();++Byte)for(unsigned Bit=0;Bit<8;++Bit){Fixture F;F.W.Data[Byte]^=1u<<Bit;run(F);}
 for(size_t Byte=0;Byte<8;++Byte)for(unsigned V=0;V<256;++V){Fixture F;F.W.Data[F.Offset+Byte]=V;run(F);}
 // Overflow, unterminated and noncanonical VBR must stop without shifting UB.
 unsigned char V[16];for(auto &B:V)B=255;CheckedBits Bits(V,0,128);uint64_t Value;assert(!Bits.vbr(32,Value));
 unsigned char NonCanonical[2]={128,0};CheckedBits NC(NonCanonical,0,16);assert(!NC.vbr(8,Value));
 std::puts("PASS: canonical record, timestamps, flags, paths, limits, truncation, overflow, all-bit and offset-byte mutations");
}
