<script setup lang="ts">
import { computed, ref } from "vue";
import type { PendingPlan, Subtask, TaskMain } from "../types";

const props = defineProps<{
  task: TaskMain;
  subtasks: Subtask[];
  humanTarget: Subtask | null;
  designGateSubtask: Subtask | null;
  pendingHumanSubtask: Subtask | null;
  pendingPlan: PendingPlan | null;
  canHuman: boolean;
  busy: boolean;
  humanStageLabel: string;
  humanActionLabel: string;
  humanReason: string;
}>();

const emit = defineEmits<{
  (e: "run"): void;
  (e: "resume"): void;
  (e: "followup"): void;
  (e: "approve-plan"): void;
  (e: "reject-plan"): void;
  (e: "approve-design"): void;
  (e: "reject-design"): void;
  (e: "force-pass"): void;
  (e: "reject"): void;
  (e: "extend", rounds: number): void;
  (e: "global-force-pass"): void;
  (e: "global-rework"): void;
}>();

const extraRounds = ref(2);
const isIntake = computed(() => props.task.task_phase === "requirement_intake");
const isCompleted = computed(() => props.task.status === "completed");
const isRunning = computed(
  () => props.task.status === "running" || props.task.status === "planning"
);
const isPlanApproval = computed(() => props.task.task_phase === "plan_approval");
const isGlobal = computed(() => props.task.task_phase === "global_review");
const isDesignAwaiting = computed(() => !!props.designGateSubtask);
const actionBusy = computed(() => props.busy);

function extend() {
  emit("extend", Number(extraRounds.value));
}
</script>

<template>
  <div class="action-dock">
        <template v-if="task.status === 'terminated'">
          <div class="human-panel terminated-panel">
            <div class="hp-head">
              <span class="badge badge-red">任务已终止</span>
              <b>可以继续当前任务</b>
              <span v-if="pendingHumanSubtask" class="hp-round">
                继续后仍需处理模块 #{{ pendingHumanSubtask.sort_index }}
              </span>
            </div>
            <div class="hp-text">
              终止不会删除任何数据：已完成模块不会重跑，当前模块轮次、diff、评审意见和日志都会保留。
              点击「继续任务」从当前持久化进度恢复。
            </div>
            <div class="hp-actions">
              <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('resume')">
                继续任务
              </button>
            </div>
          </div>
        </template>
        <template
          v-else-if="
            isIntake && (task.status === 'paused' || task.status === 'pending')
          "
        >
          <div class="human-panel">
            <div class="hp-head">
              <span class="badge badge-yellow">
                {{ task.status === "pending" ? "等待启动" : "已暂停" }}
              </span>
              <b>追加需求已保留，尚未开始梳理</b>
            </div>
            <div class="hp-text">
              点击下方按钮继续，评审会重新梳理本次追加需求；不会丢失已输入内容。
            </div>
            <div class="hp-actions">
              <button
                class="btn btn-sm btn-primary"
                :disabled="actionBusy"
                @click="task.status === 'pending' ? emit('run') : emit('resume')"
              >
                {{ task.status === "pending" ? "运行" : "恢复梳理" }}
              </button>
            </div>
          </div>
        </template>
        <template v-else-if="isIntake && task.status !== 'failed'">
          <div class="human-panel completed-panel">
            <div class="hp-head">
              <span class="badge badge-blue">需求梳理中</span>
              <b>评审正在判断本次改动规模</b>
              <span class="hp-round">
                {{ task.followup_mode === "single" ? "单模块快改" : "自动判断" }}
              </span>
            </div>
            <div class="hp-text">
              评审会沿用现有技术栈与全局契约，只梳理这次新增内容，不会重新拆解已完成项目。
              简单改动只创建 1 个子任务；复杂需求才会拆成少量模块。
            </div>
            <div v-if="task.followup_requirement" class="hp-reason">
              <div class="hp-label">本次追加需求</div>
              <div class="hp-reason-text">{{ task.followup_requirement }}</div>
            </div>
          </div>
        </template>
        <template v-else-if="task.status === 'failed'">
          <div class="human-panel">
            <div class="hp-head">
              <span class="badge badge-red">执行失败</span>
              <b>{{ isIntake ? "追加需求梳理未完成" : "任务执行中断" }}</b>
            </div>
            <div class="hp-text">
              {{
                isIntake
                  ? "本次追加需求仍保留，点击「运行」可重新让评审梳理，不会丢失已输入内容。"
                  : "历史数据与当前进度均保留，可点击「运行」重试当前步骤。"
              }}
            </div>
            <div v-if="isIntake && task.followup_requirement" class="hp-reason">
              <div class="hp-label">待梳理的追加需求</div>
              <div class="hp-reason-text">{{ task.followup_requirement }}</div>
            </div>
            <div class="hp-actions">
              <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('run')">
                运行 / 重试
              </button>
            </div>
          </div>
        </template>
        <template v-else-if="isCompleted">
          <div class="human-panel completed-panel">
            <div class="hp-head">
              <span class="badge badge-green">✅ 任务已完成</span>
              <b>可以继续提出新的问题或需求</b>
              <span class="hp-round">共 {{ subtasks.length }} 个模块</span>
            </div>
            <div class="hp-text">
              所有模块已通过全局评审。你可以继续输入新的需求，评审者会先梳理增量：
              <b>简单改动只创建 1 个子任务</b>，避免重复拆解和过度设计；复杂需求才拆成少量模块。
            </div>
            <div class="hp-actions">
              <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('followup')">
                ＋ 追加需求，继续迭代
              </button>
            </div>
            <div class="hp-hint">
              已有代码、模块摘要和评审历史全部保留，不会重跑已完成模块。
            </div>
          </div>
        </template>
        <template v-else-if="isPlanApproval">
          <div class="human-bar">
            <div class="tip">只有你同意方案后，Coder 才会开始工作。</div>
            <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('approve-plan')">
              同意方案，开始开发
            </button>
            <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="emit('reject-plan')">
              驳回，让技术总监修改
            </button>
          </div>
        </template>
        <template v-else-if="isDesignAwaiting">
          <div class="human-bar">
            <div class="tip">设计文档审批通过后，该模块才会派发给 Coder。</div>
            <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('approve-design')">
              同意设计，派发 Coder
            </button>
            <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="emit('reject-design')">
              驳回设计
            </button>
          </div>
        </template>
        <template v-else-if="isGlobal">
          <div v-if="task.status === 'needs_human'" class="human-bar">
            <div class="tip">全局评审轮次已耗尽：可强制通过收口，或指定模块返工。</div>
            <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="emit('global-force-pass')">
              人工强制通过全局评审
            </button>
            <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="emit('global-rework')">
              人工指定模块返工
            </button>
          </div>
        </template>
        <template v-else-if="humanTarget">
          <div class="human-panel intervention-panel">
            <div class="hp-head">
              <span class="badge badge-red">需要人工介入</span>
              <b>#{{ humanTarget.sort_index }} {{ humanTarget.subtask_name }}</b>
              <span class="hp-round">
                第 {{ humanTarget.current_round }}/{{ humanTarget.max_round }} 轮 ·
                {{ humanStageLabel }}
              </span>
            </div>
            <div class="hp-reason">
              <div class="hp-label">最近评审意见</div>
              <div class="hp-reason-text">{{ humanReason }}</div>
            </div>
            <div class="hp-actions">
              <button
                class="btn btn-sm btn-primary"
                :disabled="!canHuman || actionBusy"
                @click="emit('force-pass')"
              >
                {{ humanActionLabel }}
              </button>
              <button
                class="btn btn-sm btn-danger"
                :disabled="!canHuman || actionBusy"
                @click="emit('reject')"
              >
                驳回并重做
              </button>
              <span class="hp-extend">
                <select v-model.number="extraRounds" aria-label="追加轮次">
                  <option :value="1">追加 1 轮</option>
                  <option :value="2">追加 2 轮</option>
                  <option :value="3">追加 3 轮</option>
                  <option :value="5">追加 5 轮</option>
                </select>
                <button
                  class="btn btn-sm"
                  :disabled="!canHuman || actionBusy"
                  @click="extend()"
                >
                  追加轮次继续
                </button>
              </span>
            </div>
            <div class="hp-hint">
              人工通过会直接跳过当前评审；追加轮次会保留全部历史记录，并从失败阶段继续自动执行。
            </div>
          </div>
        </template>
        <template v-else-if="task.status === 'needs_human'">
          <div class="human-panel">
            <div class="hp-head"><span class="badge badge-red">需要人工介入</span></div>
            <div class="hp-text">
              当前任务等待人工处理；请检查左侧模块或全局评审区域。
            </div>
          </div>
        </template>
        <template v-else-if="!isRunning">
          <div class="human-panel subtle-panel">
            <div class="hp-text">
              当前没有待人工处理的评审结果。可以点击顶部「运行 / 恢复 / 继续任务」。
            </div>
          </div>
        </template>          
  </div>
</template>
