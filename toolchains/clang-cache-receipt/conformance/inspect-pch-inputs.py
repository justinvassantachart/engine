#!/usr/bin/env python3
"""Bounded, test-only LLVM22 PCH input metadata reader; never admission authority."""
import hashlib, json, struct, sys
from pathlib import Path

class Reader:
    def __init__(self, data):
        if data[:4] != b'CPCH' or len(data) > 256 * 1024 * 1024:
            raise ValueError('unsupported PCH magic or size')
        self.data, self.pos, self.global_abbrevs = data, 32, {}
        self.records, self.steps = [], 0
    def bits(self, count):
        if not 0 <= count <= 64 or self.pos + count > len(self.data) * 8:
            raise ValueError('bit range')
        offset, shift = divmod(self.pos, 8)
        value = int.from_bytes(self.data[offset:offset + (shift + count + 7)//8], 'little') >> shift
        self.pos += count
        return value & ((1 << count) - 1)
    def vbr(self, width):
        if not 2 <= width <= 32: raise ValueError('VBR width')
        result, shift = 0, 0
        while True:
            value = self.bits(width)
            result |= (value & ((1 << (width-1))-1)) << shift
            if not value >> (width-1): return result
            shift += width-1
            if shift > 64: raise ValueError('VBR overflow')
    def align(self): self.pos = (self.pos + 31) & ~31
    def abbreviation(self):
        count = self.vbr(5)
        if count > 128: raise ValueError('abbreviation cap')
        result = []
        for _ in range(count):
            if self.bits(1): result.append(('literal', self.vbr(8)))
            else:
                encoding = self.bits(3)
                if encoding not in (1,2,3,4,5): raise ValueError('unknown encoding')
                result.append((encoding, self.vbr(5) if encoding in (1,2) else None))
        return result
    def operand(self, encoding, width):
        start = self.pos
        if encoding == 'literal': return width, None
        if encoding == 1: return self.bits(width), start
        if encoding == 2: return self.vbr(width), start
        if encoding == 4:
            value = self.bits(6)
            return ord('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._'[value]), start
        raise ValueError('unexpected scalar encoding')
    def record(self, abbreviation):
        values, starts, blob, blob_start = [], [], b'', None
        index = 0
        while index < len(abbreviation):
            encoding, width = abbreviation[index]
            if encoding == 3:
                count = self.vbr(6)
                if count > 1024*1024 or index + 2 != len(abbreviation): raise ValueError('array cap/shape')
                for _ in range(count):
                    value, start = self.operand(*abbreviation[index+1]); values.append(value); starts.append(start)
                index += 1
            elif encoding == 5:
                count = self.vbr(6)
                if count > 1024*1024: raise ValueError('blob cap')
                self.align(); blob_start = self.pos
                end = self.pos + count*8
                if end > len(self.data)*8: raise ValueError('blob range')
                blob = self.data[self.pos//8:end//8]; self.pos = end; self.align()
            else:
                value, start = self.operand(encoding, width); values.append(value); starts.append(start)
            index += 1
        if not values: raise ValueError('empty record')
        return values[0], values[1:], starts[1:], blob, blob_start
    def block(self, block_id, width, end):
        abbreviations = list(self.global_abbrevs.get(block_id, []))
        target = None
        while self.pos < end:
            self.steps += 1
            if self.steps > 200000: raise ValueError('record step cap')
            start = self.pos
            code = self.bits(width)
            if code == 0:
                self.align()
                if self.pos != end and block_id != -1: raise ValueError('block extent mismatch')
                return
            if code == 1:
                child = self.vbr(8); child_width = self.vbr(4); self.align()
                words = self.bits(32); child_end = self.pos + words*32
                if child_end > end or not 2 <= child_width <= 32: raise ValueError('child range')
                if child in (0,15,16): self.block(child, child_width, child_end)
                self.pos = child_end
            elif code == 2:
                abbreviation = self.abbreviation()
                if block_id == 0:
                    if target is None: raise ValueError('missing block-info id')
                    self.global_abbrevs.setdefault(target, []).append(abbreviation)
                else: abbreviations.append(abbreviation)
            else:
                if code == 3:
                    record_code = self.vbr(6); count = self.vbr(6)
                    if count > 1024*1024: raise ValueError('record cap')
                    values, starts = [], []
                    for _ in range(count): starts.append(self.pos); values.append(self.vbr(6))
                    blob, blob_start = b'', None
                else:
                    index = code-4
                    if index >= len(abbreviations): raise ValueError('unknown abbreviation')
                    record_code, values, starts, blob, blob_start = self.record(abbreviations[index])
                if block_id == 0:
                    if record_code == 1:
                        if len(values) != 1: raise ValueError('block-info id shape')
                        target = values[0]
                else:
                    self.records.append(dict(block=block_id, code=record_code, values=values, valueBitOffsets=starts, blob=blob, blobBitOffset=blob_start, recordBitOffset=start))


def inspect(file):
    data = Path(file).read_bytes(); reader = Reader(data); reader.block(-1,2,len(data)*8)
    metadata = [r for r in reader.records if r['block']==15 and r['code']==1]
    if len(metadata)!=1 or len(metadata[0]['values'])<7: raise ValueError('metadata shape')
    offsets = [r for r in reader.records if r['block']==15 and r['code']==5]
    if len(offsets)!=1 or len(offsets[0]['values'])!=2: raise ValueError('input offsets shape')
    num_inputs,num_user = offsets[0]['values']
    if num_inputs>16384 or num_user>num_inputs or len(offsets[0]['blob'])!=num_inputs*8: raise ValueError('input offset range')
    records = [r for r in reader.records if r['block']==16 and r['code']==1]
    hashes = [r for r in reader.records if r['block']==16 and r['code']==2]
    if len(records)!=num_inputs or len(hashes)!=num_inputs: raise ValueError('input count')
    inputs=[]
    for index,r in enumerate(records):
        v=r['values']
        if len(v)!=8 or v[0]!=index+1 or v[7]>len(r['blob']) or any(value not in (0,1) for value in v[3:7]): raise ValueError('input record shape')
        requested=r['blob'][:v[7]].decode('utf8'); actual=r['blob'][v[7]:].decode('utf8') or requested
        inputs.append(dict(id=v[0],size=v[1],storedTime=v[2],overridden=bool(v[3]),transient=bool(v[4]),topLevel=bool(v[5]),moduleMap=bool(v[6]),nameAsRequested=requested,name=actual,contentHash=hashes[index]['values'],valueBitOffsets=r['valueBitOffsets'],recordBitOffset=r['recordBitOffset']))
    m=metadata[0]
    return dict(schema=1,tool='test-only bounded bitstream reader for pinned LLVM22; not compiler receipt authority',pchSha256=hashlib.sha256(data).hexdigest(),bytes=len(data),hasTimestamps=bool(m['values'][6]),metadata=m['values'],metadataValueBitOffsets=m['valueBitOffsets'],numInputs=num_inputs,numUserInputs=num_user,imports=sum(r['block']==15 and r['code']==2 for r in reader.records),inputs=inputs,metadataIndependent=(not m['values'][6] and all(not i['storedTime'] and not i['overridden'] and not i['transient'] and not i['moduleMap'] for i in inputs)))

if __name__=='__main__':
    print(json.dumps(inspect(sys.argv[1]),indent=2))
