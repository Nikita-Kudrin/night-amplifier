import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref, nextTick} from 'vue'
import AstapInstallOverlay from './AstapInstallOverlay.vue'

// Mock the API module
vi.mock('../composables/api.js', () => ({
    getAstapStatus: vi.fn(),
    getAstapDatabases: vi.fn(),
    installAstap: vi.fn(),
}))

import {getAstapStatus, getAstapDatabases, installAstap} from '../composables/api.js'

describe('AstapInstallOverlay', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        getAstapStatus.mockResolvedValue({
            binary_installed: false,
            database_installed: false,
            installed_databases: [],
            ready: false,
        })
        getAstapDatabases.mockResolvedValue([
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
        installAstap.mockResolvedValue({})
    })

    function createMockEventStream(astapInstallProgressRef) {
        return {
            astapInstallProgress: astapInstallProgressRef,
            clearAstapInstallProgress: vi.fn(),
        }
    }

    function mountOverlay(astapInstallProgressRef = ref(null), props = {}) {
        return mount(AstapInstallOverlay, {
            props,
            global: {
                provide: {
                    eventStream: createMockEventStream(astapInstallProgressRef),
                },
            },
        })
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
            const d80Checkbox = wrapper.find('input[value="D80"]')
            expect(d80Checkbox.element.checked).toBe(true)
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
            getAstapDatabases.mockResolvedValue([
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
            getAstapStatus.mockResolvedValue({
                binary_installed: true,
                database_installed: true,
                installed_databases: [{id: 'D80', database_path: '/astap/d80_database'}],
                ready: true,
            })

            const wrapper = mountOverlay(ref(null), {allowManage: true})
            await flushPromises()

            expect(wrapper.text()).toContain('Installed')
        })
    })

    describe('Installation progress display', () => {
        it('shows progress text with percentage during download', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            // Start installation
            wrapper.vm.installing = true
            await nextTick()

            // Simulate progress event
            progressRef.value = {
                component: 'W08 Database',
                stage: 'downloading',
                percent: 4.15,
                bytesDownloaded: 52428800, // 50 MB
                totalBytes: 1261887242, // ~1.2 GB
                stageName: 'Downloading Database',
                overallPercent: 52.08,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.installing-state').exists()).toBe(true)
            expect(wrapper.find('.progress-text').text()).toContain('Downloading W08 Database')
            expect(wrapper.find('.progress-text').text()).toContain('50.0')
            expect(wrapper.find('.progress-text').text()).toContain('1203.4')
            expect(wrapper.find('.progress-text').text()).toContain('4.2%')
        })

        it('shows progress bar with correct width', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 25.5,
                bytesDownloaded: 100000000,
                totalBytes: 400000000,
                overallPercent: 62.75,
            }
            await nextTick()
            await flushPromises()

            const progressFill = wrapper.find('.progress-fill')
            expect(progressFill.exists()).toBe(true)
            // Overall percent is used for progress bar
            expect(progressFill.attributes('style')).toContain('width: 62.75%')
        })

        it('shows overall progress text', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 10,
                bytesDownloaded: 100000000,
                totalBytes: 1000000000,
                overallPercent: 55,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.overall-progress-text').text()).toContain('Overall progress: 55%')
        })

        it('shows extracting progress', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'D80 Database',
                stage: 'extracting',
                percent: 45.5,
                stageName: 'Extracting Database',
                overallPercent: 72.75,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('Extracting D80 Database')
            expect(wrapper.find('.progress-text').text()).toContain('45.5%')
        })

        it('shows completed state', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'D80 Database',
                stage: 'completed',
                stageName: 'Database Installed',
                overallPercent: 100,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('D80 Database installed successfully')
        })

        it('shows failed state with error message', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'D80 Database',
                stage: 'failed',
                error: 'Connection timeout',
            }
            await nextTick()
            await flushPromises()

            // When failed, installing becomes false and error is shown
            expect(wrapper.vm.installing).toBe(false)
            expect(wrapper.find('.error-state').exists()).toBe(true)
            expect(wrapper.find('.error-message').text()).toContain('Installation failed: Connection timeout')
        })
    })

    describe('Progress updates via eventStream', () => {
        it('updates display when astapInstallProgress changes', async () => {
            const astapInstallProgress = ref(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: {
                        eventStream: {
                            astapInstallProgress,
                            clearAstapInstallProgress: vi.fn(),
                        },
                    },
                },
            })
            await flushPromises()

            // Start installation
            wrapper.vm.installing = true
            await nextTick()

            // Simulate progress event
            astapInstallProgress.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 15.3,
                bytesDownloaded: 200000000,
                totalBytes: 1300000000,
                stageName: 'Downloading Database',
                overallPercent: 57.65,
            }
            await nextTick()
            // Allow watch to trigger
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('Downloading D80 Database')
            expect(wrapper.find('.progress-text').text()).toContain('15.3%')
        })

        it('updates stage completion tracking during download', async () => {
            const astapInstallProgress = ref(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: {
                        eventStream: {
                            astapInstallProgress,
                            clearAstapInstallProgress: vi.fn(),
                        },
                    },
                },
            })
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            // CLI download
            astapInstallProgress.value = {
                component: 'ASTAP CLI',
                stage: 'downloading',
                percent: 50,
                bytesDownloaded: 50000000,
                totalBytes: 100000000,
                stageName: 'Downloading ASTAP CLI',
                overallPercent: 25,
            }
            await nextTick()
            await flushPromises()

            // Verify CLI stage is active
            const stageItems = wrapper.findAll('.stage-item')
            expect(stageItems[0].classes()).toContain('active')
            expect(stageItems[0].classes()).not.toContain('completed')
        })

        it('marks CLI stage completed when database download starts', async () => {
            const astapInstallProgress = ref(null)
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: {
                        eventStream: {
                            astapInstallProgress,
                            clearAstapInstallProgress: vi.fn(),
                        },
                    },
                },
            })
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            // Database download (implies CLI is done)
            astapInstallProgress.value = {
                component: 'D80 Database',
                stage: 'downloading',
                percent: 5,
                bytesDownloaded: 50000000,
                totalBytes: 1000000000,
                stageName: 'Downloading Database',
                overallPercent: 52.5,
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
            expect(wrapper.vm.installing).toBe(true)
        })
    })

    describe('Close behavior', () => {
        it('emits close event when cancel clicked', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            const cancelButton = wrapper.find('.btn-secondary')
            await cancelButton.trigger('click')

            expect(wrapper.emitted('close')).toBeTruthy()
        })

        it('clears progress on unmount', async () => {
            const clearAstapInstallProgress = vi.fn()
            const wrapper = mount(AstapInstallOverlay, {
                global: {
                    provide: {
                        eventStream: {
                            astapInstallProgress: ref(null),
                            clearAstapInstallProgress,
                        },
                    },
                },
            })
            await flushPromises()

            wrapper.unmount()

            expect(clearAstapInstallProgress).toHaveBeenCalled()
        })
    })

    describe('Manage mode', () => {
        it('does not auto-close when allowManage is true and system is ready', async () => {
            getAstapStatus.mockResolvedValue({
                binary_installed: true,
                database_installed: true,
                installed_databases: [{id: 'D80', database_path: '/astap/d80_database'}],
                ready: true,
            })
            getAstapDatabases.mockResolvedValue([
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

            const wrapper = mountOverlay(ref(null), {allowManage: true})
            await flushPromises()

            // Should NOT have emitted close
            expect(wrapper.emitted('close')).toBeFalsy()
            // Should show database section for managing
            expect(wrapper.find('.database-section').exists()).toBe(true)
        })

        it('shows "Download Selected" button in manage mode', async () => {
            getAstapStatus.mockResolvedValue({
                binary_installed: true,
                database_installed: true,
                installed_databases: [{id: 'D80', database_path: '/astap/d80_database'}],
                ready: true,
            })
            getAstapDatabases.mockResolvedValue([
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

            const wrapper = mountOverlay(ref(null), {allowManage: true})
            await flushPromises()

            const installBtn = wrapper.find('.btn-primary')
            expect(installBtn.text()).toBe('Download Selected')
        })

        it('shows all-installed message when everything is downloaded', async () => {
            getAstapStatus.mockResolvedValue({
                binary_installed: true,
                database_installed: true,
                installed_databases: [
                    {id: 'D80', database_path: '/astap/d80_database'},
                    {id: 'G05', database_path: '/astap/g05_database'},
                    {id: 'W08', database_path: '/astap/w08_database'},
                ],
                ready: true,
            })
            getAstapDatabases.mockResolvedValue([
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

            const wrapper = mountOverlay(ref(null), {allowManage: true})
            await flushPromises()

            expect(wrapper.text()).toContain('All available databases are installed')
        })
    })
})
