/** Help for the Push-To panel and its equipment section. */
export const PUSH_TO_HELP = {
    push_to_without_optics:
        'Push-To might not work reliably. By specifying the focal length and sensor pixel size, you can increase the success rate of Push-To.',
    push_to_pro: 'Plate solving and Push-To navigation require Night Amplifier Pro.',
    telescope_focal_length: 'The focal length of your telescope in millimeters.',
    telescope_camera_sensor:
        'Select your camera from the database or enter pixel size manually. You can also auto-fill from the connected camera.',
    telescope_barlow:
        'Barlow/reducer coefficient. 1.0 = no barlow/reducer. Values > 1.0 for barlows (e.g. 2.0 for 2x barlow). Values < 1.0 for reducers (e.g. 0.63 for a 0.63x reducer).',
    telescope_fov:
        'Calculated Field of View based on your telescope and camera parameters. Sent to the plate solver for faster solving.',
}
