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
            ready: false,
        })
        getAstapDatabases.mockResolvedValue([
            {id: 'W08', description: 'Wide Field', fov_range: '0.8° - 8°', size: '90 MB'},
            {id: 'G05', description: 'Gaia DR3', fov_range: '0.2° - 5°', size: '400 MB'},
        ])
        installAstap.mockResolvedValue({})
    })

    function createMockEventStream(astapInstallProgressRef) {
        return {
            astapInstallProgress: astapInstallProgressRef,
            clearAstapInstallProgress: vi.fn(),
        }
    }

    function mountOverlay(astapInstallProgressRef = ref(null)) {
        return mount(AstapInstallOverlay, {
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
            expect(wrapper.text()).toContain('Select Star Database')
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
                component: 'W08 database',
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
            expect(wrapper.find('.progress-text').text()).toContain('Downloading W08 database')
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
                component: 'W08 database',
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
                component: 'W08 database',
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
                component: 'W08 database',
                stage: 'extracting',
                percent: 45.5,
                stageName: 'Extracting Database',
                overallPercent: 72.75,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('Extracting W08 database')
            expect(wrapper.find('.progress-text').text()).toContain('45.5%')
        })

        it('shows completed state', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'W08 database',
                stage: 'completed',
                stageName: 'Database Installed',
                overallPercent: 100,
            }
            await nextTick()
            await flushPromises()

            expect(wrapper.find('.progress-text').text()).toContain('W08 database installed successfully')
        })

        it('shows failed state with error message', async () => {
            const progressRef = ref(null)
            const wrapper = mountOverlay(progressRef)
            await flushPromises()

            wrapper.vm.installing = true
            await nextTick()

            progressRef.value = {
                component: 'W08 database',
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
                component: 'W08 database',
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

            expect(wrapper.find('.progress-text').text()).toContain('Downloading W08 database')
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
                component: 'W08 database',
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
        it('starts installation when clicked', async () => {
            const wrapper = mountOverlay()
            await flushPromises()

            // Select W08 database (first in list)
            const w08Option = wrapper.find('input[value="W08"]')
            await w08Option.setValue(true)

            const installButton = wrapper.find('.btn-primary')
            await installButton.trigger('click')
            await flushPromises()

            expect(installAstap).toHaveBeenCalledWith('W08')
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
})
