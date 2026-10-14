import {describe, it, expect, expectTypeOf, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import type {ComponentMountingOptions, VueWrapper} from '@vue/test-utils'
import {ref, nextTick} from 'vue'
import type {Ref} from 'vue'
import AstapInstallOverlay from './AstapInstallOverlay.vue'
import type AstapStatusSection from './AstapStatusSection.vue'
import {createInstallEvents} from '../composables/events/installEvents'
import type {AstapInstallProgress} from '../composables/events/installEvents'
import {createEventStreamStub, sendTo} from '../test/eventStream'
import type {AstapStatus, InstalledDatabaseInfo} from '../composables/api.types'

// Mock the API module
vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
        getAstapStatus: vi.fn(),
        getAstapDatabases: vi.fn(),
        installAstap: vi.fn(),
    }))

import {getAstapStatus, getAstapDatabases, installAstap} from '../composables/api'
import {contextProvides} from '../composables/appContext'

const D80: InstalledDatabaseInfo = {id: 'D80', database_path: '/astap/d80_database', min_fov_deg: 0.15, max_fov_deg: 6.0}
const G05: InstalledDatabaseInfo = {id: 'G05', database_path: '/astap/g05_database', min_fov_deg: 3.0, max_fov_deg: 20.0}
const W08: InstalledDatabaseInfo = {id: 'W08', database_path: '/astap/w08_database', min_fov_deg: 20.0, max_fov_deg: 80.0}

/** A ready solver with these databases installed. */
function ready(...installed: InstalledDatabaseInfo[]): AstapStatus {
    return {binary_installed: true, database_installed: true, installed_databases: installed, ready: true}
}

describe('AstapInstallOverlay', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(getAstapStatus).mockResolvedValue({
            binary_installed: false,
            database_installed: false,
            installed_databases: [],
            ready: false,
        })
        vi.mocked(getAstapDatabases).mockResolvedValue([
            {
                id: 'D80',
                description: 'General Purpose',
                min_fov_deg: 0.15,
                max_fov_deg: 6.0,
                size: '~1.3GB',
                installed: false
            },
            {
                id: 'G05',
                description: 'Camera Lenses',
                min_fov_deg: 3.0,
                max_fov_deg: 20.0,
                size: '~100MB',
                installed: false
            },
            {
                id: 'W08',
                description: 'Fisheye Lenses',
                min_fov_deg: 20.0,
                max_fov_deg: 80.0,
                size: '<1MB',
                installed: false
            },
        ])
        vi.mocked(installAstap).mockResolvedValue({message: 'Installation started'})
    })

    function mountOverlay(
        astapInstallProgressRef = ref<AstapInstallProgress | null>(null),
        props: {allowManage?: boolean} = {},
    ) {
        return mount(AstapInstallOverlay, {
            props,
            global: {
                provide: contextProvides({
                    eventStream: createEventStreamStub({astapInstallProgress: astapInstallProgressRef}),
                }),
            },
        })
    }

    /** Starts an install of what is selected, as its button does. */
    async function startInstalling(wrapper: VueWrapper) {
        await wrapper.find('.btn-primary').trigger('click')
        await nextTick()
    }

    describe('Initial state', () => {
        it('shows loading state initially', () => {
            const wrapper = mountOverlay()
            expect(wrapper.find('.loading-state').exists()).toBe(true)
            expect(wrapper.text()).toContain('Checking installation status')
        })

        it('shows database selection after loading', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            expect(wrapper.find('.loading-state').exists()).toBe(false)
            expect(wrapper.find('.database-section').exists()).toBe(true)
            expect(wrapper.text()).toContain('Select Star Databases')
        })

        it('pre-selects D80 for fresh install', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            // D80 should be pre-selected via checkbox
            const d80Checkbox = wrapper.find<HTMLInputElement>('input[value="D80"]')
            expect(d80Checkbox.element.checked).toBe(true)
        })

        it('lists what a fresh solver lacks', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            const rows = wrapper.findAll('.status-section .status-item').map((row) => row.text())
            expect(rows).toEqual(['✗ASTAP CLI', '✗No Star Database'])
            // The section reads a whole status, which the overlay has once it loaded.
            expectTypeOf<NonNullable<ComponentMountingOptions<typeof AstapStatusSection>['props']>['status']>()
                .toEqualTypeOf<AstapStatus>()
        })
    })

    describe('Multi-database selection', () => {
        it('uses checkboxes instead of radio buttons', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            const checkboxes = wrapper.findAll('input[type="checkbox"]')
            expect(checkboxes.length).toBe(3)
            // No radio buttons
            expect(wrapper.findAll('input[type="radio"]').length).toBe(0)
        })

        it('shows installed badge for installed databases', async () => {
            vi.mocked(getAstapDatabases).mockResolvedValue([
                {
                    id: 'D80',
                    description: 'General Purpose',
                    min_fov_deg: 0.15,
                    max_fov_deg: 6.0,
                    size: '~1.3GB',
                    installed: true
                },
                {
                    id: 'G05',
                    description: 'Camera Lenses',
                    min_fov_deg: 3.0,
                    max_fov_deg: 20.0,
                    size: '~100MB',
                    installed: false
                },
                {
                    id: 'W08',
                    description: 'Fisheye Lenses',
                    min_fov_deg: 20.0,
                    max_fov_deg: 80.0,
                    size: '<1MB',
                    installed: false
                },
            ])
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))

            const wrapper = mountOverlay(undefined, {allowManage: true})
            await flushPromises()

            expect(wrapper.text()).toContain('Installed')
        })
    })

    describe('Installation progress display', () => {
        it('shows progress text with percentage during download', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            // Start installation
            await startInstalling(wrapper)

            // Simulate progress event
            progressRef.value = {
                component: 'W08 Database',
                stage: 'downloading',
                percent: 4.15,
                bytesDownloaded: 52428800, // 50 MB
                totalBytes: 1261887242, // ~1.2 GB
                stageId: 'downloading_database',
                overallPercent: 52.08,
                error: null,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.installing-state').exists()).toBe(true)
            expect(wrapper.find('.progress-text').text()).toContain('Downloading W08 Database')
            expect(wrapper.find('.progress-text').text()).toContain('50.0 MB / 1.18 GB (4.2%)')
        })

        it('shows progress bar with correct width', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            await startInstalling(wrapper)

            progressRef.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 25.5,
                bytesDownloaded: 100000000,
                totalBytes: 400000000,
                overallPercent: 62.75,
                error: null,
            }
            await nextTick()
            await flushPromises()

            const progressFill = wrapper.find('.progress-fill')
            expect(progressFill.exists()).toBe(true)
            // Overall percent is used for progress bar
            expect(progressFill.attributes('style')).toContain('width: 62.75%')
        })

        it('shows overall progress text', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            await startInstalling(wrapper)

            progressRef.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 10,
                bytesDownloaded: 100000000,
                totalBytes: 1000000000,
                overallPercent: 55,
                error: null,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.overall-progress-text').text()).toContain('Overall progress: 55%')
        })

        it('shows extracting progress', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            await startInstalling(wrapper)

            progressRef.value = {
                component: 'D80 Database',
                stage: 'extracting',
                percent: 45.5,
                stageId: 'extracting_database',
                overallPercent: 72.75,
                error: null,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('Extracting D80 Database')
            expect(wrapper.find('.progress-text').text()).toContain('45.5%')
        })

        it('shows completed state', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            await startInstalling(wrapper)

            progressRef.value = {
                component: 'D80 Database',
                stage: 'completed',
                stageId: 'database_completed',
                overallPercent: 100,
                error: null,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('D80 Database installed successfully')
        })

        it('shows failed state with error message', async () => {
            const progressRef = ref<AstapInstallProgress | null>(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            await startInstalling(wrapper)

            progressRef.value = {
                component: 'D80 Database',
                stage: 'failed',
                error: 'Connection timeout',
            }
            await nextTick()
            await flushPromises()

            // When failed, installing becomes false and error is shown
            expect(wrapper.find('.installing-state').exists()).toBe(false)
            expect(wrapper.find('.error-state').exists()).toBe(true)
            expect(wrapper.find('.error-message').text()).toContain('Installation failed: Connection timeout')
        })
    })

    describe('Progress updates via eventStream', () => {
        it('updates display when astapInstallProgress changes', async () => {
            const astapInstallProgress = ref<AstapInstallProgress | null>(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: contextProvides({
                        eventStream: createEventStreamStub({astapInstallProgress}),
                    }),
                },
            })
            await flushPromises()

            // Start installation
            await startInstalling(wrapper)

            // Simulate progress event
            astapInstallProgress.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 15.3,
                bytesDownloaded: 200000000,
                totalBytes: 1300000000,
                stageId: 'downloading_database',
                overallPercent: 57.65,
                error: null,
            }
            await nextTick()
            // Allow watch to trigger
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('Downloading D80 Database')
            expect(wrapper.find('.progress-text').text()).toContain('15.3%')
        })

        it('updates stage completion tracking during download', async () => {
            const astapInstallProgress = ref<AstapInstallProgress | null>(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: contextProvides({
                        eventStream: createEventStreamStub({astapInstallProgress}),
                    }),
                },
            })
            await flushPromises()

            await startInstalling(wrapper)

            // CLI download
            astapInstallProgress.value = {
                component: 'ASTAP CLI',
                stage: 'downloading',
                percent: 50,
                bytesDownloaded: 50000000,
                totalBytes: 100000000,
                stageId: 'downloading_cli',
                overallPercent: 25,
                error: null,
            }
            await nextTick()
            await flushPromises()

            // Verify CLI stage is active
            const stageItems = wrapper.findAll('.stage-item')
            expect(stageItems[0].classes()).toContain('active')
            expect(stageItems[0].classes()).not.toContain('completed')
        })

        it('marks CLI stage completed when database download starts', async () => {
            const astapInstallProgress = ref<AstapInstallProgress | null>(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: contextProvides({
                        eventStream: createEventStreamStub({astapInstallProgress}),
                    }),
                },
            })
            await flushPromises()

            await startInstalling(wrapper)

            // Database download (implies CLI is done)
            astapInstallProgress.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 5,
                bytesDownloaded: 50000000,
                totalBytes: 1000000000,
                stageId: 'downloading_database',
                overallPercent: 52.5,
                error: null,
            }
            await nextTick()
            await flushPromises()

            // Verify CLI stage is completed and database stage is active
            const stageItems = wrapper.findAll('.stage-item')
            expect(stageItems[0].classes()).toContain('completed')
            expect(stageItems[1].classes()).toContain('active')
        })
    })

    describe('Install button', () => {
        it('starts installation with selected databases', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            // D80 is pre-selected, also select G05
            const g05Checkbox = wrapper.find('input[value="G05"]')
            await g05Checkbox.setValue(true)

            const installButton = wrapper.find('.btn-primary')
            await installButton.trigger('click')
            await flushPromises()

            expect(installAstap).toHaveBeenCalledWith(['D80', 'G05'])
            expect(wrapper.find('.installing-state').exists()).toBe(true)
        })
    })

    describe('Close behavior', () => {
        it('emits close event when cancel clicked', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            const cancelButton = wrapper.find('.btn-ghost')
            await cancelButton.trigger('click')

            expect(wrapper.emitted('close')).toBeTruthy()
        })

        it('clears progress on unmount', async () => {
            const eventStream = createEventStreamStub()
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: contextProvides({eventStream}),
                },
            })
            await flushPromises()

            wrapper.unmount()

            expect(eventStream.clearAstapInstallProgress).toHaveBeenCalled()
        })
    })

    describe('Settling after an install', () => {
        let wrapper: VueWrapper

        afterEach(() => vi.useRealTimers())

        async function completeInstall(progressRef: Ref<AstapInstallProgress | null>) {
            await flushPromises()
            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()
            progressRef.value = {component: 'D80 Database', stage: 'completed', stageId: 'database_completed', overallPercent: 100, error: null}
            await flushPromises()
        }

        it('reloads the status and closes once the install settled', async () => {
            vi.useFakeTimers()
            const progressRef = ref<AstapInstallProgress | null>(null)
            wrapper = mountOverlay(progressRef)
            await completeInstall(progressRef)
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))

            await vi.advanceTimersByTimeAsync(500)
            await flushPromises()

            expect(getAstapStatus).toHaveBeenCalledTimes(2)
            expect(wrapper.emitted('installed')).toHaveLength(1)
            expect(wrapper.emitted('close')).toHaveLength(1)
        })

        it('closes once when opened on a solver that is already ready', async () => {
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))
            wrapper = mountOverlay()
            await flushPromises()

            expect(wrapper.emitted('installed')).toHaveLength(1)
            expect(wrapper.emitted('close')).toHaveLength(1)
        })

        it('announces a managed install once and stays open', async () => {
            vi.useFakeTimers()
            const progressRef = ref<AstapInstallProgress | null>(null)
            wrapper = mountOverlay(progressRef, {allowManage: true})
            await completeInstall(progressRef)
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))

            await vi.advanceTimersByTimeAsync(500)
            await flushPromises()

            expect(wrapper.emitted('installed')).toHaveLength(1)
            expect(wrapper.emitted('close')).toBeUndefined()
        })

        it('closes once a sync says the install whose end it missed completed', async () => {
            vi.useFakeTimers()
            const {handlers, state} = createInstallEvents()
            wrapper = mountOverlay(state.astapInstallProgress)
            await flushPromises()
            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))

            sendTo(handlers, 'astap_install_status', {state: 'completed'})
            await vi.advanceTimersByTimeAsync(500)
            await flushPromises()

            expect(wrapper.emitted('installed')).toHaveLength(1)
            expect(wrapper.emitted('close')).toHaveLength(1)
        })

        it('stays put when a connect finds no install running', async () => {
            vi.useFakeTimers()
            const {handlers, state} = createInstallEvents()
            wrapper = mountOverlay(state.astapInstallProgress)
            await flushPromises()

            sendTo(handlers, 'astap_install_status', {state: 'idle'})
            await vi.advanceTimersByTimeAsync(500)
            await flushPromises()

            expect(getAstapStatus).toHaveBeenCalledTimes(1)
            expect(wrapper.emitted('installed')).toBeUndefined()
            expect(wrapper.find('.btn-primary').attributes('disabled')).toBeUndefined()
        })

        it('drops the status reload when it goes away first', async () => {
            vi.useFakeTimers()
            const progressRef = ref<AstapInstallProgress | null>(null)
            wrapper = mountOverlay(progressRef)
            await completeInstall(progressRef)
            vi.mocked(getAstapStatus).mockClear()

            wrapper.unmount()
            await vi.advanceTimersByTimeAsync(500)

            expect(getAstapStatus).not.toHaveBeenCalled()
        })
    })

    describe('Manage mode', () => {
        it('does not auto-close when allowManage is true and system is ready', async () => {
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))
            vi.mocked(getAstapDatabases).mockResolvedValue([
                {
                    id: 'D80',
                    description: 'General Purpose',
                    min_fov_deg: 0.15,
                    max_fov_deg: 6.0,
                    size: '~1.3GB',
                    installed: true
                },
                {
                    id: 'G05',
                    description: 'Camera Lenses',
                    min_fov_deg: 3.0,
                    max_fov_deg: 20.0,
                    size: '~100MB',
                    installed: false
                },
            ])

            const wrapper = mountOverlay(undefined, {allowManage: true})
            await flushPromises()

            // Should NOT have emitted close
            expect(wrapper.emitted('close')).toBeFalsy()
            // Should show database section for managing
            expect(wrapper.find('.database-section').exists()).toBe(true)
        })

        it('shows "Download Selected" button in manage mode', async () => {
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80))
            vi.mocked(getAstapDatabases).mockResolvedValue([
                {
                    id: 'D80',
                    description: 'General Purpose',
                    min_fov_deg: 0.15,
                    max_fov_deg: 6.0,
                    size: '~1.3GB',
                    installed: true
                },
                {
                    id: 'G05',
                    description: 'Camera Lenses',
                    min_fov_deg: 3.0,
                    max_fov_deg: 20.0,
                    size: '~100MB',
                    installed: false
                },
            ])

            const wrapper = mountOverlay(undefined, {allowManage: true})
            await flushPromises()

            const installBtn = wrapper.find('.btn-primary')
            expect(installBtn.text()).toBe('Download Selected')
        })

        it('shows all-installed message when everything is downloaded', async () => {
            vi.mocked(getAstapStatus).mockResolvedValue(ready(D80, G05, W08))
            vi.mocked(getAstapDatabases).mockResolvedValue([
                {
                    id: 'D80',
                    description: 'General Purpose',
                    min_fov_deg: 0.15,
                    max_fov_deg: 6.0,
                    size: '~1.3GB',
                    installed: true
                },
                {
                    id: 'G05',
                    description: 'Camera Lenses',
                    min_fov_deg: 3.0,
                    max_fov_deg: 20.0,
                    size: '~100MB',
                    installed: true
                },
                {
                    id: 'W08',
                    description: 'Fisheye Lenses',
                    min_fov_deg: 20.0,
                    max_fov_deg: 80.0,
                    size: '<1MB',
                    installed: true
                },
            ])

            const wrapper = mountOverlay(undefined, {allowManage: true})
            await flushPromises()

            expect(wrapper.text()).toContain('All available databases are installed')
        })
    })
})
