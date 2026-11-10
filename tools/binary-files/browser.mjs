// Real Chromium Run/Debug coverage for the shared opaque runtime file mount.
import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const root = resolve(import.meta.dirname, '../..');
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const sentinel = [0, 255, 128, 192, 10, 13, 0, 42];
const programs = {
  python: `import os, base64\nexpected = bytes([0,255,128,192,10,13,0,42])\ndata = open('/images/雪.bin', 'rb').read()\nassert data == expected\nassert '雪.bin' in os.listdir('/images')\nassert base64.b64decode(base64.b64encode(data)) == expected\nopen('/images/result.bin', 'wb').write(data)\nassert open('/images/result.bin', 'rb').read() == expected\nprint('BINARY PASS')\n`,
  c: `#include <fstream>\n#include <iostream>\n#include <vector>\nint main() {\n  std::ifstream input("/images/雪.bin", std::ios::binary);\n  std::vector<unsigned char> data((std::istreambuf_iterator<char>(input)), {});\n  if (data != std::vector<unsigned char>{0,255,128,192,10,13,0,42}) return 1;\n  std::ofstream out("/images/result.bin", std::ios::binary);\n  out.write(reinterpret_cast<const char*>(data.data()), data.size()); out.close();\n  std::ifstream reload("/images/result.bin", std::ios::binary);\n  std::vector<unsigned char> copy((std::istreambuf_iterator<char>(reload)), {});\n  if (copy != data) return 2;\n  std::cout << "BINARY PASS" << std::endl;\n}\n`
};
const html = `<meta charset="utf-8"><script type="module">import {Engine} from '/engine.js';
window.start = async ({lang,source,bytes,debug,bad}) => {
 const engine=await Engine.create(lang);window.engine=engine;
 const state={stdout:'',stderr:'',stops:0,done:false};window.state=state;
 const file=lang==='python'?'main.py':'main.cpp'; engine.fs={[file]:source};
 engine.binaryFiles={'/images/雪.bin':new Uint8Array(bytes)};
 if(bad==='collision') engine.binaryFiles['/'+file]=new Uint8Array([1]);
 if(bad==='traversal') engine.binaryFiles['/images/../escape']=new Uint8Array([1]);
 if(bad==='alias') engine.binaryFiles['/images//alias']=new Uint8Array([1]);
 if(bad==='relative') engine.binaryFiles['images/relative']=new Uint8Array([1]);
 if(bad==='artifacts') engine.cppArtifacts={archives:[]};
 const decoder=new TextDecoder();engine.stdout.on('data',b=>state.stdout+=decoder.decode(b));engine.stderr.on('data',b=>state.stderr+=decoder.decode(b));
 let seq=0; const send=(command,args={})=>engine.debugger.send({type:'request',seq:++seq,command,arguments:args});
 engine.debugger.enabled=debug;
 if(debug) engine.debugger.on('event',event=>queueMicrotask(()=>{
  if(event.event==='initialized'){send('setBreakpoints',{source:{path:'/'+file},breakpoints:[{line:lang==='python'?4:7}]});send('configurationDone');}
  if(event.event==='stopped'){state.stops++;send('continue',{threadId:1});}
 }));
 engine.run().then(result=>{state.result=result;state.done=true},error=>{state.error=String(error);state.done=true});
 engine.binaryFiles['/images/雪.bin'].fill(17);
 if(debug)send('initialize',{adapterID:'binary-files'});
};</script>`;
const assets = new Map([['/engine.js', await readFile(resolve(root,'dist/debugger-sh.js'))],
 ['/assets/cpp-exceptions/runtime.tar.gz',await readFile(resolve(root,'assets/cpp-exceptions/runtime.tar.gz'))]]);
const server=createServer((request,response)=>{
 response.setHeader('Cross-Origin-Opener-Policy','same-origin');response.setHeader('Cross-Origin-Embedder-Policy','require-corp');
 if(request.url==='/'){response.setHeader('Content-Type','text/html');response.end(html);}
 else if(assets.has(request.url)){response.setHeader('Content-Type',request.url.endsWith('.js')?'text/javascript':'application/octet-stream');response.end(assets.get(request.url));}
 else response.writeHead(404).end();
});
let browser;const results=[];const errors=[];
try {
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));browser=await chromium.launch({headless:true});
 const page=await browser.newPage();page.on('pageerror',error=>errors.push(String(error)));
 for(const lang of ['python','c']) for(const scenario of [false,true,'collision','traversal','alias','relative',...(lang==='python'?['artifacts']:[])]) {
  await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>window.start);
  await page.evaluate(options=>window.start(options),{lang,source:programs[lang],bytes:sentinel,debug:scenario===true,bad:typeof scenario==='string'?scenario:undefined});
  await page.waitForFunction(()=>window.state?.done,undefined,{timeout:120000});const state=await page.evaluate(()=>window.state);
  if(typeof scenario==='string'){assert.equal(state.result?.type,'error',JSON.stringify(state));assert.match(state.result.error.message,/binary input|Binary build inputs/i);}
  else {assert.equal(state.result?.type,'completed',JSON.stringify(state));assert.equal(state.result.exitCode,0,JSON.stringify(state));assert.match(state.stdout,/BINARY PASS/);if(scenario)assert.ok(state.stops>0);}
  results.push({lang,scenario,...state});console.log(JSON.stringify({lang,scenario,result:state.result,stops:state.stops}));
 }
 assert.deepEqual(errors,[]);await mkdir(resolve(root,'tools/binary-files/output'),{recursive:true});
 await writeFile(resolve(root,'tools/binary-files/output/browser.json'),JSON.stringify({browser:await browser.version(),results,errors},null,2)+'\n');
} finally {await browser?.close();await new Promise(resolve=>server.close(resolve));}
