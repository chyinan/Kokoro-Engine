// Isolated native regression; requires run-chat-lifecycle-native.ps1 -Persistence.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
const out = path.resolve(process.argv[2] ?? 'output/playwright/chat-lifecycle/persistence');
const db = path.join(process.env.KOKORO_TEST_DATA_ROOT, 'com.chyin.kokoro', 'kokoro.db');
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE);
const character = 'chat-persistence-validation-character';
const calls = [];
const runId = Date.now().toString();
let sttMode = 'original';
const server = http.createServer(async (request, response) => {
  let raw = ''; for await (const chunk of request) raw += chunk;
  if (request.url.endsWith('/audio/transcriptions')) {
    calls.push({kind: 'stt', mode: sttMode});
    response.writeHead(sttMode === 'unavailable' ? 400 : 200, {'content-type':'application/json'});
    response.end(JSON.stringify(sttMode === 'unavailable' ? {error:{message:'controlled STT unavailable'}} : {text:sttMode === 'original' ? 'original transcript' : 'original transcript.'}));
    return;
  }
  const body = JSON.parse(raw);
  const memory = body.messages?.some(x => typeof x.content === 'string' && x.content.includes('You are a memory extraction assistant'));
  const background = body.messages?.some(x=>typeof x.content==='string' && x.content.startsWith('Character reply: '));
  calls.push({kind:memory ? 'memory' : background ? 'background' : body.stream ? 'chat' : 'other', messages:body.messages});
  if (body.stream) {
    response.writeHead(200, {'content-type':'text/event-stream'});
    response.end('data: '+JSON.stringify({id:'audit',object:'chat.completion.chunk',created:1,model:'audit',choices:[{index:0,delta:{role:'assistant',content:'audit reply'},finish_reason:null}]})+'\n\ndata: [DONE]\n\n');
  } else {
    response.writeHead(200, {'content-type':'application/json'});
    response.end(JSON.stringify({id:'audit',object:'chat.completion',created:1,model:'audit',choices:[{index:0,message:{role:'assistant',content:memory?'[]':background?'{"should_generate":false}':'{"cues":[]}'},finish_reason:'stop'}]}));
  }
});
await new Promise(resolve => server.listen(19336, '127.0.0.1', resolve));
const browser = await chromium.connectOverCDP('http://127.0.0.1:19333');
const page = browser.contexts()[0].pages().find(x => x.url().includes('pet.html'));
const invoke = async (command, args) => {
  const result = await page.evaluate(async ({command,args})=>{
    try {return {ok:true,value:await window.__TAURI_INTERNALS__.invoke(command,args)};}
    catch(error){return {ok:false,error};}
  },{command,args});
  if (!result.ok) throw new Error(`${command}: ${JSON.stringify(result.error)}`);
  return result.value;
};
const wait = ms => new Promise(resolve=>setTimeout(resolve,ms));
const count = kind => calls.filter(x=>x.kind===kind).length;
const sql = query => execFileSync('python',['-c','import sqlite3,sys\nwith sqlite3.connect(sys.argv[1],timeout=10) as db: db.executescript(sys.argv[2])',db,query]);
const rows = query => JSON.parse(execFileSync('python',['-c','import sqlite3,json,sys\nwith sqlite3.connect(sys.argv[1],timeout=10) as db:\n db.row_factory=sqlite3.Row\n print(json.dumps([dict(x) for x in db.execute(sys.argv[2])]))',db,query],{encoding:'utf8'}));
const webhook = async (id, text) => {
  id = runId + '-' + id;
  const response = await fetch('http://127.0.0.1:19335/webhook/message',{method:'POST',headers:{Authorization:'Bearer fixture-chat-token','content-type':'application/json'},body:JSON.stringify({text,request_id:id,character_id:character,conversation_id:'review-audio',source:'review-audit',audio_base64:'UklGRg==',audio_format:'wav'})});
  return {status:response.status,body:await response.json()};
};
const report = {head:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),db,runId};
try {
  await invoke('save_llm_config',{config:{active_provider:'fixture',system_provider:'fixture',providers:[{id:'fixture',provider_type:'openai',enabled:true,supports_native_tools:false,api_key:'local-controlled-fixture',base_url:'http://127.0.0.1:19336/v1',model:'audit',extra:{}}]}});
  await invoke('save_stt_config',{config:{active_provider:'audit',providers:[{id:'audit',provider_type:'openai_whisper',enabled:true,api_key:'local-controlled-fixture',base_url:'http://127.0.0.1:19336/v1',model:'audit'}]}});
  const botConfig=await invoke('get_bot_config'); botConfig.webhook.send_voice_reply=false; await invoke('save_bot_config',{config:botConfig});
  // The server snapshots config at startup, but existing local speech fixture remains isolated.
  report.committedFirst=await webhook('audit-committed');
  assert.equal(report.committedFirst.status,200);
  sttMode='drift'; report.committedDrift=await webhook('audit-committed');
  assert.deepEqual(report.committedDrift,report.committedFirst);
  sttMode='unavailable'; report.committedUnavailable=await webhook('audit-committed');
  assert.deepEqual(report.committedUnavailable,report.committedFirst);
  sttMode='original';
  sql("CREATE TRIGGER audit_reject_external BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' AND NEW.content='audit reply' BEGIN SELECT RAISE(ABORT,'audit external final failure'); END;");
  try { report.pendingFirst=await webhook('audit-pending'); } finally {sql('DROP TRIGGER audit_reject_external;');}
  assert.equal(report.pendingFirst.body.code,'CHAT_PERSISTENCE_ERROR');
  const generatedBefore=count('chat'); sttMode='drift'; report.pendingDrift=await webhook('audit-pending');
  assert.equal(report.pendingDrift.status,200);
  sttMode='original'; report.pendingOriginal=await webhook('audit-pending');
  assert.equal(report.pendingOriginal.status,200); assert.equal(count('chat'),generatedBefore);
  report.audioCalls={stt:count('stt'),generation:count('chat')};
  assert.deepEqual(report.audioCalls,{stt:2,generation:2});
  const changedInput = await webhook('audit-committed','changed caption');
  assert.equal(changedInput.status,409);
  assert.match(changedInput.body.error,/chat_request_conflict/);
  sttMode='unavailable'; assert.equal((await webhook('audit-first-failure')).status,500);
  sttMode='original'; assert.equal((await webhook('audit-first-failure')).status,200);
  report.audioRows=rows("SELECT role,content,metadata FROM conversation_messages WHERE content IN ('original transcript','original transcript.','audit reply')");
  const upgrade=await invoke('get_memory_upgrade_config'); upgrade.event_trigger_enabled=false;
  await invoke('set_memory_upgrade_config',{config:upgrade});
  await invoke('set_memory_enabled',{enabled:true});
  await invoke('clear_history');
  const submit=id=>invoke('stream_chat',{request:{client_request_id:runId+'-'+id,character_id:character,message:id,allow_image_gen:false}});
  const memoryBefore=count('memory');
  for(let i=1;i<=5;i++) assert.equal((await submit('audit-control-'+i)).status,'completed');
  await wait(700);
  report.controlMemoryExtractions=count('memory')-memoryBefore;
  assert.equal(report.controlMemoryExtractions,1);
  await invoke('clear_history'); await invoke('set_memory_enabled',{enabled:true});
  const failedBefore=count('memory');
  for(let i=1;i<=4;i++) await submit('audit-retry-'+i);
  sql("CREATE TRIGGER audit_reject_desktop BEFORE UPDATE ON conversation_messages WHEN NEW.role='assistant' AND NEW.content='audit reply' AND json_extract(NEW.metadata,'$.streaming_draft') IS NULL BEGIN SELECT RAISE(ABORT,'audit desktop final failure'); END;");
  try {
    report.desktopFifthError = await page.evaluate(async request => {
      try {return {unexpectedSuccess:await window.__TAURI_INTERNALS__.invoke('stream_chat',{request})};} catch(error){return error;}
    }, {client_request_id:runId+'-audit-retry-5',character_id:character,message:'audit-retry-5',allow_image_gen:false});
    assert.equal(report.desktopFifthError.code,'ChatPersistence');
  } finally {sql('DROP TRIGGER audit_reject_desktop;');}
  report.desktopRetry=await invoke('retry_chat_persistence',{clientRequestId:runId+'-audit-retry-5'});
  assert.equal(report.desktopRetry.status,'completed');
  await wait(700); report.recoveredMemoryExtractions=count('memory')-failedBefore;
  assert.equal(report.recoveredMemoryExtractions,1);
  await invoke('retry_chat_persistence',{clientRequestId:runId+'-audit-retry-5'});
  await wait(200); assert.equal(count('memory')-failedBefore,1);
  report.desktopRecoveredRow=rows('SELECT role,content,metadata FROM conversation_messages WHERE id='+report.desktopRetry.assistant_message_id);
  const consolidationCount = () => rows("SELECT COUNT(*) AS n FROM memory_write_events WHERE source='chat' AND trigger='periodic_consolidation'")[0].n;
  const failFinal = () => sql("CREATE TRIGGER audit_reject_desktop BEFORE UPDATE ON conversation_messages WHEN NEW.role='assistant' AND NEW.content='audit reply' AND json_extract(NEW.metadata,'$.streaming_draft') IS NULL BEGIN SELECT RAISE(ABORT,'audit desktop final failure'); END;");
  async function failedTurn(id, message=id, allowImage=false) {
    failFinal();
    try {
      const error=await page.evaluate(async request=>{try{return {unexpectedSuccess:await window.__TAURI_INTERNALS__.invoke('stream_chat',{request})};}catch(error){return error;}}, {client_request_id:runId+'-'+id,character_id:character,message,allow_image_gen:allowImage});
      assert.equal(error.code,'ChatPersistence');
    } finally {sql('DROP TRIGGER audit_reject_desktop;');}
  }
  const retry = id=>invoke('retry_chat_persistence',{clientRequestId:runId+'-'+id});
  await invoke('clear_history'); await invoke('set_memory_enabled',{enabled:true});
  const twentiethMemory=count('memory'), consolidationBefore=consolidationCount();
  for(let i=1;i<20;i++) await submit('audit-twentieth-'+i);
  await wait(300); assert.equal(count('memory')-twentiethMemory,3);
  await failedTurn('audit-twentieth-20');
  await Promise.all([retry('audit-twentieth-20'),retry('audit-twentieth-20')]);
  await wait(700);
  assert.equal(count('memory')-twentiethMemory,4);
  assert.equal(consolidationCount()-consolidationBefore,1);
  await retry('audit-twentieth-20'); await wait(200);
  assert.equal(consolidationCount()-consolidationBefore,1);
  report.twentieth={extractions:4,consolidations:1};

  await invoke('clear_history'); await invoke('set_memory_enabled',{enabled:true});
  upgrade.event_trigger_enabled=true; upgrade.event_cooldown_secs=1;
  await invoke('set_memory_upgrade_config',{config:upgrade});
  const eventBefore=count('memory');
  await failedTurn('audit-event','I like jasmine tea.');
  await retry('audit-event'); await retry('audit-event'); await wait(700);
  assert.equal(count('memory')-eventBefore,1);
  report.eventExtractions=1;

  await invoke('clear_history'); await invoke('set_memory_enabled',{enabled:true});
  const toggledBefore=count('memory');
  await failedTurn('audit-toggle','I like jasmine tea.');
  await invoke('set_memory_enabled',{enabled:false}); await invoke('set_memory_enabled',{enabled:true});
  assert.equal((await retry('audit-toggle')).status,'completed'); await wait(500);
  assert.equal(count('memory'),toggledBefore); report.oldEpochSkipped=true;

  await invoke('clear_history');
  const switchedBefore=count('memory'), switchedBackgroundBefore=count('background');
  const switchedCalls=calls.length;
  await failedTurn('audit-switch','I like jasmine tea. audit-switch-owner',true);
  const bound = (await invoke('get_chat_runtime_snapshot')).find(turn=>turn.client_request_id===runId+'-audit-switch');
  await invoke('clear_history');
  const recoveredSwitch=await retry('audit-switch');
  assert.equal(recoveredSwitch.status,'completed');
  assert.equal(recoveredSwitch.conversation_id,bound.conversation_id);
  for(let attempts=0;attempts<100 && (count('memory')===switchedBefore || count('background')===switchedBackgroundBefore);attempts++) await wait(50);
  assert.equal(count('memory')-switchedBefore,1);
  assert.equal(count('background')-switchedBackgroundBefore,1);
  assert.equal((await invoke('get_chat_view')).conversation_id,null);
  const switchedRows=rows("SELECT m.id,m.role,m.content,m.metadata,c.character_id,m.conversation_id FROM conversation_messages m JOIN conversations c ON c.id=m.conversation_id WHERE m.conversation_id="+JSON.stringify(bound.conversation_id));
  assert(switchedRows.some(row=>row.id===recoveredSwitch.assistant_message_id && row.content==='audit reply'));
  assert(switchedRows.some(row=>row.role==='user' && row.content==='I like jasmine tea. audit-switch-owner'));
  assert(switchedRows.every(row=>row.character_id===character));
  const extraction=calls.slice(switchedCalls).filter(call=>call.kind==='memory');
  assert.equal(extraction.length,1);
  assert(JSON.stringify(extraction[0].messages).includes('audit-switch-owner'));
  assert.deepEqual(await retry('audit-switch'),recoveredSwitch);
  await wait(200);
  assert.equal(count('memory')-switchedBefore,1);
  assert.equal(count('background')-switchedBackgroundBefore,1);
  report.originalOwnerCompletedAfterSwitch={conversation:bound.conversation_id,extractions:1,backgroundAnalyses:1,rows:switchedRows};

  // Background analysis shares recovery, but has an independent task from memory.
  const otherBefore=count('background');
  await failedTurn('audit-background','neutral message',true);
  await retry('audit-background'); await retry('audit-background'); await wait(500);
  assert.equal(count('background')-otherBefore,1); report.backgroundAnalyses=1;
  report.status='passed';
  console.log('PASS recovered media bypasses preparation, rejects changed text, and preserves first-failure retry');
  console.log('PASS desktop recovery dispatches fifth/twentieth/event memory once, fences revoked memory epochs, preserves switched owners, and restores background analysis');

} catch(error) {
  report.status='failed'; report.error=error.stack ?? String(error); throw error;
} finally {
  fs.writeFileSync(path.join(out,'recovery-report.json'),JSON.stringify(report,null,2));
  fs.writeFileSync(path.join(out,'recovery-model-calls.json'),JSON.stringify(calls,null,2));
  await browser.close(); await new Promise(resolve=>server.close(resolve));
}
