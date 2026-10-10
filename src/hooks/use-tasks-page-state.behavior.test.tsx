import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { useTasksPageState } from "./use-tasks-page-state";
import * as api from "@/lib/tauri-api";
import { defaultSettings, defaultAsrConfig } from "@/config";
import type { TaskRecord, SettingsState } from "@/types";
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./use-app-resume", () => ({ useAppResume: () => {} }));
vi.mock("@/lib/tauri-api", () => ({ cancelTask:vi.fn(),checkEnvironment:vi.fn(),createAudioTask:vi.fn(),createSrtTask:vi.fn(),createVideoTask:vi.fn(),deleteTask:vi.fn(),listTasks:vi.fn(),loadQueueSettings:vi.fn(),loadSettings:vi.fn(),runTaskOperation:vi.fn(),runTaskOperations:vi.fn(),saveQueueSettings:vi.fn(),selectOutputDir:vi.fn(),selectAudio:vi.fn(),selectSrt:vi.fn(),selectVideo:vi.fn() }));
let state: ReturnType<typeof useTasksPageState>;
let renderer: ReactTestRenderer | undefined;
const translate = (key: string) => key;
function Probe(){state=useTasksPageState(translate);return null;}
beforeEach(()=>{
 vi.clearAllMocks(); vi.useFakeTimers(); vi.stubGlobal("window", {__TAURI_INTERNALS__:{},setTimeout,clearTimeout});
 vi.mocked(api.listTasks).mockResolvedValue([]);
 vi.mocked(api.loadQueueSettings).mockResolvedValue({max_concurrency:2,auto_start_next:false});
 vi.mocked(api.checkEnvironment).mockResolvedValue({ffmpeg_path:"/ffmpeg",whisper_path:null,gpu_name:null,cuda_driver:null,resource_dir:"",config_dir:"",sidecar_dir:"",model_dir:""});
 vi.mocked(api.selectVideo).mockResolvedValue("/fixture/fresh.mp4");
});
afterEach(()=>{act(()=>renderer?.unmount());renderer=undefined;vi.useRealTimers();vi.unstubAllGlobals();});
describe("fresh optional-engine queue flow",()=>{
 for(const engine of ["whisper-accelerated","qwen3-asr"]){
  it(`loads fresh ${engine} settings, creates the selected-engine task and queues with no legacy model or executable`,async()=>{
   const settings:SettingsState={...defaultSettings,whisper_model_path:"",asr:{...defaultAsrConfig,engine,python_path:"/fixture/python",model_path:"/fixture/model",aligner_path:"/fixture/aligner",device:"cpu"}};
   vi.mocked(api.loadSettings).mockResolvedValue(settings);
   const record:TaskRecord={id:"fresh",source_type:"video",video_path:"/fixture/fresh.mp4",file_name:"fresh.mp4",status:"idle",stage:"created",message:"",progress:0,settings,result_revision:0,created_at:1,updated_at:1};
   vi.mocked(api.createVideoTask).mockResolvedValue(record);
   await act(async()=>{renderer=create(<Probe/>);});
   await act(async()=>{await vi.advanceTimersByTimeAsync(400);});
   await act(async()=>{await state.createVideoTask();});
   expect(api.createVideoTask).toHaveBeenCalledWith(expect.objectContaining({whisper_model_path:"",asr:settings.asr}));
   expect(state.tasks[0].settings.asr?.engine).toBe(engine);
   await act(async()=>{await state.runOperation("fresh","transcribe");});
   expect(api.runTaskOperation).toHaveBeenCalledWith("fresh","transcribe");
   expect(state.notice).toBe("notice.addedToQueue");
  });
 }
});
