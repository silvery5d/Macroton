/* Short, low-volume diagnostic for macOS default-output hot switching.
 * This is independent of Wine and never changes the system output device.
 */
#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <math.h>
#include <stdio.h>
#include <unistd.h>

static double phase;

static OSStatus render(void *context, AudioUnitRenderActionFlags *flags,
                       const AudioTimeStamp *time, UInt32 bus, UInt32 frames,
                       AudioBufferList *output)
{
    (void)context; (void)flags; (void)time; (void)bus;
    const double step = 2.0 * M_PI * 220.0 / 48000.0;
    for (UInt32 buffer = 0; buffer < output->mNumberBuffers; ++buffer) {
        Float32 *samples = output->mBuffers[buffer].mData;
        UInt32 channels = output->mBuffers[buffer].mNumberChannels;
        for (UInt32 frame = 0; frame < frames; ++frame) {
            Float32 value = (Float32)(sin(phase) * 0.015);
            for (UInt32 channel = 0; channel < channels; ++channel)
                samples[frame * channels + channel] = value;
            phase += step;
            if (phase >= 2.0 * M_PI) phase -= 2.0 * M_PI;
        }
    }
    return noErr;
}

int main(void)
{
    AudioComponentDescription desc = {
        .componentType = kAudioUnitType_Output,
        .componentSubType = kAudioUnitSubType_DefaultOutput,
        .componentManufacturer = kAudioUnitManufacturer_Apple,
    };
    AudioComponent component = AudioComponentFindNext(NULL, &desc);
    if (!component) return fprintf(stderr, "DefaultOutput Audio Unit unavailable\n"), 1;
    AudioUnit unit = NULL;
    OSStatus status = AudioComponentInstanceNew(component, &unit);
    if (status != noErr) return fprintf(stderr, "cannot create Audio Unit: %d\n", (int)status), 1;

    AudioStreamBasicDescription format = {
        .mSampleRate = 48000.0,
        .mFormatID = kAudioFormatLinearPCM,
        .mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
        .mBytesPerPacket = 2 * sizeof(Float32),
        .mFramesPerPacket = 1,
        .mBytesPerFrame = 2 * sizeof(Float32),
        .mChannelsPerFrame = 2,
        .mBitsPerChannel = 8 * sizeof(Float32),
    };
    status = AudioUnitSetProperty(unit, kAudioUnitProperty_StreamFormat,
                                  kAudioUnitScope_Input, 0, &format, sizeof(format));
    if (status != noErr) goto failed;
    AURenderCallbackStruct callback = { .inputProc = render };
    status = AudioUnitSetProperty(unit, kAudioUnitProperty_SetRenderCallback,
                                  kAudioUnitScope_Input, 0, &callback, sizeof(callback));
    if (status != noErr) goto failed;
    status = AudioUnitInitialize(unit);
    if (status != noErr) goto failed;
    status = AudioOutputUnitStart(unit);
    if (status != noErr) goto failed;
    puts("Low-volume 220 Hz tone playing for 30 seconds; switch macOS output once.");
    fflush(stdout);
    sleep(30);
    AudioOutputUnitStop(unit);
    AudioUnitUninitialize(unit);
    AudioComponentInstanceDispose(unit);
    return 0;
failed:
    fprintf(stderr, "DefaultOutput Audio Unit failed: %d\n", (int)status);
    AudioComponentInstanceDispose(unit);
    return 1;
}
