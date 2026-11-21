// Run through cua_repl with already selected Browser Use tab handles.
// This module uses only public locator actions and read-only DOM observations.
// It changes task-owned preview fixtures, never a product Manager or server.
export async function runFeedbackRegression(tab) {
  const checks = []
  const check = (name, pass) => { checks.push({ name, result: pass ? 'PASS' : 'FAIL' }); if (!pass) throw Error(name) }
  const button = name => tab.playwright.getByRole('button', { name, exact: true })
  const footer = () => tab.playwright.getByLabel('設定操作', { exact: true }).innerText()
  await button('重設示範').click()
  await tab.playwright.getByRole('link', { name: '應用程式設定', exact: true }).click()
  await tab.playwright.getByRole('combobox', { name: '外觀', exact: true }).selectOption({ label: '淺色' })
  check('修改後立即顯示草稿狀態', (await footer()).includes('有尚未儲存'))
  await button('取消').click()
  check('取消後清除草稿提示', !(await footer()).includes('未儲存'))
  await tab.playwright.getByRole('combobox', { name: '外觀', exact: true }).selectOption({ label: '深色' })
  await button('掃描目錄').click()
  await tab.playwright.getByRole('dialog').getByRole('button', { name: '放棄變更', exact: true }).click()
  check('跨頁放棄後提示不殘留', !(await footer()).includes('未儲存'))
  await tab.playwright.getByRole('textbox', { name: '允許目錄（每行一個路徑）', exact: true }).fill('bad-alias')
  await button('儲存設定').click()
  check('無效路徑發布可操作錯誤', (await tab.playwright.getByRole('alert').innerText()).includes('相對路徑'))
  await button('取消').click()
  check('取消後錯誤清除', (await tab.playwright.getByRole('alert').innerText()) === '')
  await button('儲存設定').click()
  check('儲存後原位結果正確', (await footer()).includes('已儲存示範設定'))
  return checks
}

export async function runListParity(vue, legacy) {
  const checks = []
  const check = (name, pass) => { checks.push({ name, result: pass ? 'PASS' : 'FAIL' }); if (!pass) throw Error(name) }
  const ids = tab => tab.playwright.getByRole('row').evaluateAll(rows => rows.map(row => row.getAttribute('data-row')).filter(Boolean))
  for (const tab of [vue, legacy]) {
    await tab.playwright.getByRole('button', { name: '重設示範', exact: true }).click()
    await tab.playwright.getByRole('row').filter({ hasText: 'exampleitems-paper-26.2' }).click()
    await tab.playwright.getByRole('row').filter({ hasText: 'examplepack-folia-26.1.2' }).click({ modifiers: ['Control'] })
    check((tab === vue ? 'Vue' : '舊版') + '整列 Ctrl 多選', (await tab.playwright.getByRole('complementary', { name: '實例詳細資訊' }).innerText()).includes('已選取 2 台'))
    await tab.playwright.getByRole('button', { name: '清除選取', exact: true }).click()
  }
  for (const header of ['實例', '核心版本', 'Port', '觀察狀態', '最近操作']) {
    for (let phase = 0; phase < 3; phase++) {
      for (const tab of [vue, legacy]) await tab.playwright.getByRole('button', { name: header, exact: true }).click()
      check(`${header} 排序階段 ${phase + 1} 與舊版一致`, JSON.stringify(await ids(vue)) === JSON.stringify(await ids(legacy)))
    }
  }
  for (const tab of [vue, legacy]) {
    await tab.playwright.getByRole('row').filter({ hasText: 'exampleitems-paper-26.2' }).click()
    await tab.playwright.getByRole('button', { name: '釘選實例', exact: true }).click()
    await tab.playwright.getByText('管理操作 ▾', { exact: true }).click()
    await tab.playwright.getByRole('button', { name: '釘選上移', exact: true }).click()
  }
  check('釘選位置與舊版一致', JSON.stringify(await ids(vue)) === JSON.stringify(await ids(legacy)))
  for (const tab of [vue, legacy]) {
    await tab.playwright.getByRole('searchbox', { name: '搜尋實例', exact: true }).fill('lobby')
    check((tab === vue ? 'Vue' : '舊版') + '篩選清除隱藏目標', (await tab.playwright.getByRole('complementary', { name: '實例詳細資訊' }).innerText()).startsWith('選取實例'))
    await tab.playwright.getByRole('searchbox', { name: '搜尋實例', exact: true }).fill('')
    await tab.playwright.getByRole('row').filter({ hasText: 'exampleitems-paper-26.2' }).click()
    await tab.playwright.getByRole('row').filter({ hasText: 'exampleitems-paper-26.2' }).press('Escape')
    check((tab === vue ? 'Vue' : '舊版') + 'Esc 清除選取', (await tab.playwright.getByRole('complementary', { name: '實例詳細資訊' }).innerText()).startsWith('選取實例'))
  }
  return checks
}

export async function runEmergencyRegression(tab) {
  const checks = []
  const check = (name, pass) => { checks.push({ name, result: pass ? 'PASS' : 'FAIL' }); if (!pass) throw Error(name) }
  const button = name => tab.playwright.getByRole('button', { name, exact: true })
  await button('重設示範').click()
  await tab.playwright.getByRole('row', { name: /^SurvivalSample 生存世界/ }).click()
  await button('正常停止').press('Enter')
  await button('啟動').waitFor({ state: 'visible', timeoutMs: 4000 })
  check('正常停止後不把鍵盤焦點交給啟動', !(await button('啟動').evaluate(e => e === document.activeElement)))
  check('停止後重新啟動可用', await button('重新啟動').isEnabled())
  await button('重新啟動').press('Enter')
  await button('正常停止').waitFor({ state: 'visible', timeoutMs: 4000 })
  await button('正常停止').press('Enter')
  await button('緊急停止').press('Enter')
  check('強制停止預設取消焦點', await tab.playwright.getByRole('dialog').getByRole('button', { name: '取消', exact: true }).evaluate(e => e === document.activeElement))
  await button('強制停止').press('Enter')
  check('強制停止後保持停止', (await tab.playwright.getByRole('complementary', { name: '實例詳細資訊' }).innerText()).includes('強制停止：已完成'))
  return checks
}

export async function runSettingsParity(vue, legacy) {
  const checks = []
  const controls = tab => tab.playwright.getByRole('main').evaluate(element =>
    [...element.querySelectorAll('input,select,textarea')].map(input => ({
      id: input.id, type: input.type, value: input.value, checked: input.checked, disabled: input.disabled,
    })))
  for (const tab of [vue, legacy]) {
    await tab.playwright.getByRole('button', { name: '重設示範', exact: true }).click()
    await tab.playwright.getByRole('row').filter({ hasText: 'exampleitems-paper-26.2' }).click()
    await tab.playwright.getByRole('button', { name: '實例設定', exact: true }).click()
  }
  for (const group of [
    ['實例', ['Java 與 JVM', '停止與重啟', '自動重啟', '原生後台', '世界政策']],
    ['應用', ['顯示與語言', '掃描目錄', '初始化預設', '新建啟動範本']],
  ]) {
    if (group[0] === '應用') {
      for (const tab of [vue, legacy]) await tab.playwright.getByRole('link', { name: '應用程式設定', exact: true }).click()
    }
    for (const name of group[1]) {
      const values = []
      for (const tab of [vue, legacy]) {
        await tab.playwright.getByRole('button', { name, exact: true }).click()
        values.push(await controls(tab))
      }
      const pass = JSON.stringify(values[0]) === JSON.stringify(values[1])
      checks.push({ name: `${group[0]}設定 ${name} 欄位一致`, result: pass ? 'PASS' : 'FAIL' })
      if (!pass) throw Error(`${group[0]}設定 ${name} 欄位不一致`)
    }
  }
  return checks
}
