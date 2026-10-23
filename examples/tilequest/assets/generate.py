#!/usr/bin/env python3
"""Reproduce original Tilequest pixel art and Tiled fixtures using only Python stdlib."""
import json, struct, zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parent
W,H=128,64
pixels=bytearray(W*H*4)
GREEN=(40,79,61,255); MOSS=(59,105,69,255); DARK=(20,49,43,255); SAND=(169,145,93,255)
def paint(tile,x,y,color):
    if 0<=x<16 and 0<=y<16:
        at=(((tile//8*16+y)*W)+(tile%8*16+x))*4;pixels[at:at+4]=bytes(color)
def rect(tile,x,y,w,h,color):
    for a in range(x,x+w):
        for b in range(y,y+h):paint(tile,a,b,color)
for tile in range(32):
    if tile in [0,1,2,3,4,5,6,7,12,14]:
        base=SAND if tile in [2,3,14] else ((24,68,85,255) if tile in [4,5] else GREEN)
        rect(tile,0,0,16,16,base)
        for i in range(12):paint(tile,(i*7+tile)%16,(i*11+3)%16,MOSS if tile not in [4,5] else (34,91,106,255))
    if tile==1:
        for x,y in [(3,5),(11,10)]:rect(tile,x,y,2,3,DARK);rect(tile,x-1,y-1,3,2,(211,179,103,255))
    if tile in [4,5]:
        for y in [3,10]:
            for x in range(2,8):paint(tile,(x+tile*2+y)%16,y,(52,119,134,255))
    if tile==3:rect(tile,0,3,16,10,(153,131,85,255))
    if tile in [6,7]:
        rect(tile,1,3,14,12,DARK);rect(tile,2,2,12,11,(72,89,82,255));rect(tile,3,2,10,3,(102,116,96,255))
        if tile==7:
            for y in [6,10]:rect(tile,2,y,12,1,(43,62,60,255))
            rect(tile,7,2,1,4,DARK);rect(tile,5,7,1,3,DARK)
    if tile in [8,9,10,11]:
        for y in range(16):
            for x in range(16):
                xx=x+(tile%2)*16-16;yy=y+((tile-8)//2)*16-16
                if xx*xx+yy*yy<230:paint(tile,x,y,(25,60,43,255) if (x+y)%7<2 else (45,91,52,255))
        if tile in [8,9]:rect(tile,5,6,4,2,(72,118,61,255))
    if tile==12:rect(tile,0,0,16,16,(27,60,51,255))
    if tile==13:
        for y in range(16):
            for x in range(16):
                if (x-8)**2*0.4+(y-8)**2<25:paint(tile,x,y,(188,221,205,90))
    if tile==15:
        for y in range(3,12):rect(tile,4,y,2,1,(209,186,111,255))
        rect(tile,4,10,8,2,(209,186,111,255));rect(tile,10,3,3,3,(92,171,142,255))
    if tile in [16,17,20,21]:
        rect(tile,4,12,9,2,DARK);rect(tile,5,7,7,5,(100,89,64,255));rect(tile,6,5,5,3,(155,127,79,255))
        for y in range(2,10):
            for x in range(4,13):
                if abs(x-8)+abs(y-5)<5:paint(tile,x,y,(249,194,83,255) if tile in [16,20] else (100,221,173,255))
        paint(tile,7,3,(255,239,171,255))
    if tile in [18,19,22]:
        rect(tile,2,2,12,14,(71,82,76,255));rect(tile,5,4,6,12,(18,44,44,255));rect(tile,3,2,10,3,(112,116,94,255))
        if tile==19:rect(tile,5,4,6,11,(105,196,157,255));rect(tile,6,5,4,10,(186,229,175,255))
    if tile in [24,25,26,27]:
        rect(tile,4,13,9,2,(17,45,43,160));rect(tile,5,7,7,6,(48,137,130,255));rect(tile,4,3,9,5,(79,67,45,255));rect(tile,5,5,7,4,(219,190,136,255));rect(tile,3,3,11,2,(168,131,75,255));rect(tile,8,7,4,1,(60,62,42,255));paint(tile,10,6,(25,46,43,255));rect(tile,5,12+(tile%2),2,2,(96,76,46,255));rect(tile,10,13-(tile%2),2,2,(96,76,46,255));rect(tile,4,9,2,3,(212,179,124,255));rect(tile,11,9,2,3,(212,179,124,255))
    if tile==28:rect(tile,6,2,5,13,(91,74,49,255));rect(tile,7,3,1,10,(135,107,63,255))
    if tile==29:
        for x in [3,7,11]:rect(tile,x,9-x%3,1,5,(89,132,74,255))
    if tile==30:rect(tile,2,10,12,4,(9,29,29,120))
    if tile==31:rect(tile,4,5,8,8,(105,133,112,255));rect(tile,6,7,4,4,(167,184,130,255))
def chunk(kind,data):return struct.pack('!I',len(data))+kind+data+struct.pack('!I',zlib.crc32(kind+data)&0xffffffff)
raw=b''.join(b'\0'+pixels[y*W*4:(y+1)*W*4]for y in range(H))
(ROOT/'atlas.png').write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!2I5B',W,H,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b''))
def prop(name,value,kind=None):return {'name':name,'type':kind or ('bool' if isinstance(value,bool) else 'string'),'value':value}
cols,rows=128,96
ground=[];obstacles=[0]*(cols*rows);canopy=[0]*(cols*rows);clouds=[0]*(cols*rows)
for y in range(rows):
    for x in range(cols):
        water=x<3 or y<3 or x>=cols-3 or y>=rows-3 or ((x-65)**2/90+(y-46)**2/60<1) or (56<=x<=58 and y>55 and y not in [66,67,68])
        ground.append(5 if water else (2 if (x*17+y*11)%41==0 else 1))
        if not water and (x*13+y*7)%59==0:clouds[y*cols+x]=14
for cx,cy in [(25,27),(38,40),(82,18),(102,60),(16,34),(45,70),(112,41),(76,70)]:
    obstacles[cy*cols+cx]=29
    for dy in range(2):
        for dx in range(2):canopy[(cy-1+dy)*cols+cx-1+dx]=9+dy*2+dx
for x in range(43,53):
    for y in range(14,22):
        if (x in [43,52] or y in [14,21]) and not (x in [48,49] and y==21):obstacles[y*cols+x]=8
for y in range(29,35):
    for x in range(77,85):
        if (x+y)%3==0:obstacles[y*cols+x]=7
for i in range(8):canopy[19*cols+16+i]=16|((i&1)*0x80000000)|(((i>>1)&1)*0x40000000)|(((i>>2)&1)*0x20000000)
def layer(id,name,data,**fields):return {'id':id,'name':name,'type':'tilelayer','width':cols,'height':rows,'data':data,'visible':True,'opacity':1,**fields}
objects=[{'id':1,'name':'camp','class':'spawn','point':True,'x':14*16+8,'y':16*16+8},*[{'id':i+2,'name':f'Beacon {i+1}','class':'beacon','point':True,'x':x*16+8,'y':y*16+8}for i,(x,y)in enumerate([(28,18),(96,26),(90,78)])],{'id':5,'name':'Old gate','class':'exit','x':116*16,'y':82*16,'width':16,'height':16}]
mapdata={'type':'map','version':'1.10','tiledversion':'1.12.2','orientation':'orthogonal','renderorder':'right-down','infinite':False,'width':cols,'height':rows,'tilewidth':16,'tileheight':16,'backgroundcolor':'#122d2c','properties':[prop('title','Tilequest'),prop('author','Quasar original fixture')],'tilesets':[{'firstgid':1,'source':'tilesets/island.tsj'}],'layers':[layer(1,'ground',ground),layer(2,'collision',obstacles,properties=[prop('solid',True)]),{'id':3,'name':'markers','type':'objectgroup','objects':objects},layer(4,'foreground',canopy),layer(5,'clouds',clouds,opacity=0.38,parallaxx=0.65,parallaxy=0.65)]}
(ROOT/'island.tmj').write_text(json.dumps(mapdata,separators=(',',':'))+'\n')
(ROOT/'tilesets').mkdir(exist_ok=True)
tileset={'type':'tileset','version':'1.10','name':'island','image':'../atlas.png','imagewidth':W,'imageheight':H,'tilewidth':16,'tileheight':16,'columns':8,'tilecount':32,'tiles':[{'id':4,'properties':[prop('solid',True)],'animation':[{'tileid':4,'duration':350},{'tileid':5,'duration':350}]},{'id':24,'animation':[{'tileid':i,'duration':110}for i in [24,25,26,27]]}]}
(ROOT/'tilesets'/'island.tsj').write_text(json.dumps(tileset,indent=2)+'\n')
print('Wrote original atlas and 128x96 map (49,152 stored cells).')
