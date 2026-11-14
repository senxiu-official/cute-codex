import pathlib,struct,uuid,json
def check(exe,pdb):
 data=exe.read_bytes();u16=lambda off:struct.unpack_from('<H',data,off)[0];u32=lambda off:struct.unpack_from('<I',data,off)[0]
 pe=u32(0x3c);assert data[pe:pe+4]==b'PE\0\0';count=u16(pe+6);optional=pe+24
 table=optional+u16(pe+20);directory=optional+(112 if u16(optional)==0x20b else 96)
 sections=[struct.unpack_from('<8sIIII',data,table+40*i) for i in range(count)]
 def raw(rva):
  for _,size,va,raw_size,offset in sections:
   if va<=rva<va+max(size,raw_size):return offset+rva-va
  raise AssertionError('RVA not mapped')
 debug_rva,debug_size=struct.unpack_from('<II',data,directory+6*8);debug=raw(debug_rva);identities=[]
 for off in range(debug,debug+debug_size,28):
  kind=u32(off+12);size=u32(off+16);ptr=u32(off+24)
  if kind==2 and data[ptr:ptr+4]==b'RSDS':identities.append((data[ptr+4:ptr+20],u32(ptr+20)))
 assert len(identities)==1
 blob=pdb.read_bytes();assert blob.startswith(b'Microsoft C/C++ MSF 7.00')
 block,_,_,dir_size,_,map_block=struct.unpack_from('<6I',blob,32)
 pages=(dir_size+block-1)//block
 indexes=struct.unpack_from(f'<{pages}I',blob,map_block*block)
 directory=b''.join(blob[i*block:(i+1)*block] for i in indexes)[:dir_size]
 streams=struct.unpack_from('<I',directory)[0];sizes=struct.unpack_from(f'<{streams}I',directory,4);cursor=4+4*streams;info=None
 for index,size in enumerate(sizes):
  if size==0xffffffff:continue
  count=(size+block-1)//block;pages=struct.unpack_from(f'<{count}I',directory,cursor);cursor+=4*count
  if index==1:info=b''.join(blob[p*block:(p+1)*block] for p in pages)[:size]
 assert info is not None
 age=struct.unpack_from('<I',info,8)[0];guid=info[12:28];assert identities[0]==(guid,age),(exe.name,'PDB identity mismatch')
 return {'guid':str(uuid.UUID(bytes_le=guid)),'age':age,'matchesExecutable':True}

if __name__ == '__main__':
 import argparse,hashlib
 ap=argparse.ArgumentParser();ap.add_argument('--bin-dir',type=pathlib.Path,required=True);ap.add_argument('--out',type=pathlib.Path,required=True);args=ap.parse_args()
 names=['codex','codex-app-server','codex-code-mode-host','codex-exec-server'];result={}
 for name in names:
  exe=args.bin_dir/(name+'.exe');pdb=args.bin_dir/(name.replace('-','_')+'.pdb')
  result[name]={'identity':check(exe,pdb),'exeSha256':__import__('hashlib').sha256(exe.read_bytes()).hexdigest(),'pdbSha256':__import__('hashlib').sha256(pdb.read_bytes()).hexdigest()}
 args.out.write_text(json.dumps(result,indent=2)+'\n');print('Verified four executable/PDB GUID+age pairs.')
