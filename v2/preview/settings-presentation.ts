import { reactive } from 'vue'

// The legacy interaction model publishes feedback here instead of modifying
// text nodes owned by Vue. Draft input nodes stay mounted while typing.
export const settingsPresentation = reactive({ message: '', error: '' })
