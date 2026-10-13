import {eyepieceStreamEndpoint, isEyepiecePage, liveStreamEndpoint, PAGES, WS} from './endpoints.js'

describe('endpoints', () => {
    it("names `/`'s socket per camera", () => {
        expect(liveStreamEndpoint('main')).toBe(WS.STREAM)
        expect(liveStreamEndpoint('guide')).toBe('/ws/stream?source=guide')
    })

    it('gives the quality page the lossless socket and the plain one JPEG', () => {
        expect(eyepieceStreamEndpoint(PAGES.EYEPIECE_QUALITY)).toBe('/ws/eyepiece_quality')
        expect(eyepieceStreamEndpoint(PAGES.EYEPIECE)).toBe('/ws/eyepiece')
    })

    it('recognises both eyepiece pages and nothing else', () => {
        expect(isEyepiecePage('/eyepiece')).toBe(true)
        expect(isEyepiecePage('/eyepiece_quality')).toBe(true)
        expect(isEyepiecePage('/')).toBe(false)
        expect(isEyepiecePage('/eyepiece/extra')).toBe(false)
    })
})
