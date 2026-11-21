<script setup lang="ts">
import logo from '../../gui/icons/instanceharbor.png'
</script>

<template>
<a class="skip" href="#view">跳至主要內容</a>
<div class="reviewbar" aria-label="僅供預覽的工具，不屬於正式程式"><strong>預覽工具（正式版不顯示）</strong><span>Vue 瀏覽器預覽；所有資料皆為模擬</span><div class="review-actions"><a href="./legacy.html" target="_blank" rel="noopener">開啟無框架版</a><label for="scenario">模擬情境</label><select id="scenario"><option value="normal">正常</option><option value="create-download-failure">建立：核心下載失敗一次</option><option value="failure">下一次操作失敗</option><option value="timeout">下一次停止逾時</option><option value="unknown">下一次結果未確認</option><option value="conflict">下一次儲存衝突</option><option value="disconnect">管理服務斷線</option><option value="empty">空清單</option><option value="stale">清理失效釘選</option><option value="import-permission">匯入：權限不足</option><option value="import-unrecognized">匯入：核心無法辨識</option><option value="import-external">匯入：外部程序運作中</option><option value="legacy-ready">舊版已停止且 Host 閒置</option><option value="java-stale">Java 舊路徑已不存在</option><option value="java-unrunnable">Java 存在但不能執行</option><option value="mise-duplicate">mise 同路徑重複回報</option></select><button type="button" data-action="silent-check">模擬背景核對</button><button type="button" id="reset-demo">重設示範</button><button type="button" data-action="explain-frame">正式介面／預覽工具說明</button><button type="button" data-nav="review">審查說明與功能對照</button></div></div>
<div class="app" aria-label="正式產品介面設計候選"><aside class="rail"><a href="#instances" data-nav="instances" class="brand"><img alt="" width="32" height="32" :src="logo"><span translate="no">InstanceHarbor</span></a><nav aria-label="主要導覽"><a href="#instances" data-nav="instances">實例</a><a href="#activity" data-nav="activity">操作紀錄 <span id="task-count"></span></a><a href="#settings" data-nav="settings">應用程式設定</a></nav><div class="rail-bottom"><p>管理工具</p><button type="button" data-action="onboarding">接入舊版資料</button><button type="button" data-action="quit">結束管理服務</button></div></aside>
<main id="view" tabindex="-1"></main></div>
<footer class="statusbar"><span id="connection" role="status">● 管理服務：已連線（示範）</span></footer>
<div id="announcement" class="sr-only" aria-live="polite" aria-atomic="true"></div>
<div id="modal-root"></div>
</template>
